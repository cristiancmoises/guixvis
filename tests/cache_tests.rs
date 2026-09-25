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
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("guixvis-test-{tag}-{}-{nonce}", std::process::id()));
    fs::create_dir(&dir).expect("create exclusive temp dir");
    dir
}

#[test]
fn secondary_channel_changes_snapshot_content() {
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/small.json")).unwrap();
    value["header"]["origin"] = serde_json::json!({
        "executable": "/guix/bin/guix", "system": "x86_64-linux", "verified": true,
        "mutable_package_path": false, "channels": [
            {"name":"guix", "commit":"abc"}, {"name":"extra", "commit":"def"}
        ]
    });
    let a = Index::from_doc(serde_json::from_value(value.clone()).unwrap(), 1).unwrap();
    value["header"]["origin"]["channels"][1]["commit"] = "changed".into();
    let b = Index::from_doc(serde_json::from_value(value).unwrap(), 1).unwrap();
    assert_ne!(
        blob::encode(&a),
        blob::encode(&b),
        "secondary channel identity must persist"
    );
    assert_ne!(a.snapshot_id(), b.snapshot_id());
}

#[test]
fn snapshot_token_is_content_scoped_not_process_scoped() {
    let index = fixture_index();
    let loaded = blob::decode(&blob::encode(&index), 999999).unwrap();
    assert_eq!(index.snapshot_id(), loaded.snapshot_id());
    assert_eq!(index.snapshot_id().len(), 64);
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/identity.json")).unwrap();
    let a = Index::from_doc(doc.clone(), 0).unwrap();
    doc.packages[0].inputs.retain(|id| *id != 4);
    let b = Index::from_doc(doc, 0).unwrap();
    assert_ne!(a.snapshot_id(), b.snapshot_id());
}

#[test]
fn origin_order_is_canonical_and_metadata_changes_token() {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/identity.json")).unwrap();
    doc.header.origin = fixture_origin();
    doc.header.origin.channels.push(guixvis::model::ChannelPin {
        name: "extra".into(),
        commit: "abc".into(),
    });
    let a = Index::from_doc(doc.clone(), 0).unwrap();
    doc.header.origin.channels.reverse();
    let b = Index::from_doc(doc.clone(), 123).unwrap();
    assert_eq!(a.snapshot_id(), b.snapshot_id());
    doc.packages[4].catalog = true;
    let c = Index::from_doc(doc.clone(), 0).unwrap();
    assert_ne!(b.snapshot_id(), c.snapshot_id());
    doc.packages[0].native_inputs.clear();
    let d = Index::from_doc(doc.clone(), 0).unwrap();
    assert_ne!(c.snapshot_id(), d.snapshot_id());
    doc.diagnostics.push(guixvis::model::IndexDiagnostic {
        package_id: 0,
        kind: "input".into(),
        code: "accessor-failed".into(),
        message: "Unavailable".into(),
    });
    let e = Index::from_doc(doc, 0).unwrap();
    assert_ne!(d.snapshot_id(), e.snapshot_id());
}

#[test]
fn mutable_or_incomplete_origin_never_claims_freshness() {
    let dir = temp_dir("unverified");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).unwrap();
    let mut origin = fixture_origin();
    origin.mutable_package_path = true;
    assert!(matches!(
        cache.load(Some(&origin), 0).unwrap(),
        CacheStatus::Unverified(_)
    ));
    origin.mutable_package_path = false;
    origin.system.clear();
    assert!(matches!(
        cache.load(Some(&origin), 0).unwrap(),
        CacheStatus::Unverified(_)
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn v4_remains_intact_when_v5_is_built() {
    let dir = temp_dir("migration");
    let old = dir.join("index-v4.bin");
    fs::write(&old, b"GUIV4IDX rollback fixture").unwrap();
    let cache = Cache::at(dir.clone());
    assert!(matches!(cache.load(None, 0).unwrap(), CacheStatus::Absent));
    cache.save(&fixture_index()).unwrap();
    assert_eq!(fs::read(old).unwrap(), b"GUIV4IDX rollback fixture");
    assert_eq!(cache.path().file_name().unwrap(), "index-v5.bin");
    fs::remove_dir_all(dir).unwrap();
}

fn fixture_index() -> Index {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/small.json")).unwrap();
    doc.header.origin = fixture_origin();
    Index::from_doc(doc, 42).expect("fixture index")
}

fn fixture_origin() -> guixvis::model::GuixOrigin {
    guixvis::model::GuixOrigin {
        executable: "/fixture/bin/guix".into(),
        system: "x86_64-linux".into(),
        channels: vec![guixvis::model::ChannelPin {
            name: "guix".into(),
            commit: "testcommit".into(),
        }],
        verified: true,
        mutable_package_path: false,
    }
}

#[test]
fn round_trip_save_and_load() {
    let dir = temp_dir("roundtrip");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).expect("save");

    let status = cache.load(Some(&fixture_origin()), 7).expect("load");
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
    let status = cache.load(Some(&fixture_origin()), 0).expect("load");
    assert!(matches!(status, CacheStatus::Absent));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn commit_mismatch_reports_stale() {
    let dir = temp_dir("stale");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).expect("save");

    let mut other = fixture_origin();
    other.channels[0].commit = "othercommit".into();
    let status = cache.load(Some(&other), 0).expect("load");
    assert!(matches!(status, CacheStatus::Stale { .. }));
    // Same commit accepted.
    let status = cache.load(Some(&fixture_origin()), 0).expect("load");
    assert!(matches!(status, CacheStatus::Fresh(_)));
    // Unknown origin is usable, but never called fresh.
    let status = cache.load(None, 0).expect("load");
    assert!(matches!(status, CacheStatus::Unverified(_)));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn corrupt_cache_is_quarantined_not_deleted() {
    let dir = temp_dir("corrupt");
    let cache = Cache::at(dir.clone());
    fs::write(cache.path(), b"this is not an index snapshot").expect("write garbage");

    let result = cache.load(Some(&fixture_origin()), 0);
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

#[cfg(unix)]
#[test]
fn save_does_not_follow_a_preexisting_temporary_symlink() {
    use std::os::unix::fs::symlink;

    let dir = temp_dir("symlink");
    let cache = Cache::at(dir.clone());
    let victim = dir.join("unrelated-file");
    fs::write(&victim, b"keep this content").unwrap();
    let planted = dir.join(format!("index-v5.bin.tmp-{}", std::process::id()));
    symlink(&victim, &planted).unwrap();

    cache
        .save(&fixture_index())
        .expect("save with a fresh temporary");
    assert_eq!(fs::read(&victim).unwrap(), b"keep this content");
    assert!(fs::symlink_metadata(&planted)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(matches!(
        cache.load(None, 0).unwrap(),
        CacheStatus::Unverified(_)
    ));
    fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn saved_snapshot_is_private_to_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("permissions");
    let cache = Cache::at(dir.clone());
    cache.save(&fixture_index()).unwrap();
    let mode = fs::metadata(cache.path()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn concurrent_saves_use_independent_temporary_files() {
    let dir = temp_dir("concurrent");
    let cache = Cache::at(dir.clone());
    let index = fixture_index();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| cache.save(&index))).collect();
        for handle in handles {
            handle.join().unwrap().expect("concurrent save");
        }
    });
    assert!(matches!(
        cache.load(None, 0).unwrap(),
        CacheStatus::Unverified(_)
    ));
    assert_eq!(
        fs::read_dir(&dir).unwrap().count(),
        1,
        "no abandoned temporaries"
    );
    fs::remove_dir_all(dir).ok();
}
