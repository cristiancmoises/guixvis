//! Cache round-trip, snapshot validation, and quarantine behavior.

mod common;

use std::fs;
use std::path::PathBuf;

use guixvis::blob;
use guixvis::cache::{Cache, CacheStatus};
use guixvis::index::Index;
use guixvis::model::IndexDoc;

use common::load_fixture;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("guixvis-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixture_index() -> Index {
    let doc: IndexDoc = serde_json::from_str(include_str!("fixtures/small.json")).unwrap();
    Index::from_doc(doc, 42).expect("fixture index")
}

#[test]
fn round_trip_save_and_load() {
    let dir = temp_dir("roundtrip");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).expect("save");

    let status = cache.load(Some("testcommit"), 7).expect("load");
    let CacheStatus::Fresh(loaded) = status else {
        panic!("expected Fresh, got {status:?}");
    };
    assert_eq!(loaded.guix_commit, "testcommit");
    assert_eq!(loaded.packages.len(), 10);
    assert_eq!(loaded.built_ms, 7);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn snapshot_keeps_dependency_edges_and_reverse_edges() {
    let index = fixture_index();
    let bytes = blob::encode(&index);
    let back = blob::decode(&bytes, index.built_ms).expect("decode");

    assert_eq!(back.packages.len(), index.packages.len());
    assert_eq!(back.names.len(), index.names.len());
    assert_eq!(back.by_module.len(), index.by_module.len());
    for (a, b) in index.packages.iter().zip(back.packages.iter()) {
        assert_eq!(
            (a.id, &a.name, &a.file, a.line),
            (b.id, &b.name, &b.file, b.line)
        );
        assert_eq!(a.inputs, b.inputs);
        assert_eq!(a.propagated, b.propagated);
        assert_eq!(a.native, b.native);
        assert_eq!(a.licenses, b.licenses);
    }
    // Reverse edges are rebuilt from the encoded forward lists.
    for id in 0..index.len() as u32 {
        assert_eq!(
            index.dependents_count(id),
            back.dependents_count(id),
            "dependent count for id {id}"
        );
    }
}

#[test]
fn truncated_or_garbage_snapshot_is_rejected() {
    let index = fixture_index();
    let bytes = blob::encode(&index);

    assert!(blob::decode(b"not a snapshot at all", 0).is_err());
    assert!(blob::decode(&bytes[..8], 0).is_err());
    // Cut the payload in half: the reader must fail instead of guessing.
    assert!(blob::decode(&bytes[..bytes.len() / 2], 0).is_err());
    // A length field pointing far past the end must not allocate wildly.
    let mut poisoned = bytes.clone();
    let header = 8 + 4 + 4 + 4 + index.guix_commit.len() + 8;
    poisoned[header..header + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(blob::decode(&poisoned, 0).is_err());
}

#[test]
fn absent_cache_reports_absent() {
    let dir = temp_dir("absent");
    let cache = Cache::at(dir.clone());
    let status = cache.load(Some("testcommit"), 0).expect("load");
    assert!(matches!(status, CacheStatus::Absent));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn commit_mismatch_reports_stale() {
    let dir = temp_dir("stale");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).expect("save");

    let status = cache.load(Some("othercommit"), 0).expect("load");
    assert!(matches!(status, CacheStatus::Stale { .. }));
    // Same commit accepted.
    let status = cache.load(Some("testcommit"), 0).expect("load");
    assert!(matches!(status, CacheStatus::Fresh(_)));
    // Unknown live commit (unkeyed mode) also accepted.
    let status = cache.load(None, 0).expect("load");
    assert!(matches!(status, CacheStatus::Fresh(_)));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_cache_is_quarantined_not_deleted() {
    let dir = temp_dir("corrupt");
    let cache = Cache::at(dir.clone());
    fs::write(cache.path(), b"this is not an index snapshot").expect("write garbage");

    let result = cache.load(Some("testcommit"), 0);
    assert!(result.is_err());
    cache.quarantine();
    assert!(!cache.path().exists(), "corrupt cache moved away");
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
        .collect();
    assert_eq!(leftovers.len(), 1, "quarantined file kept for evidence");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn fixture_index_builds_from_cache_doc() {
    let index = fixture_index();
    let _ = load_fixture();
    assert_eq!(index.built_ms, 42);
    assert_eq!(index.len(), 10);
}
