//! Live integration: run the real Guile indexer against the host's Guix.
//! Ignored by default; run with `cargo test -- --ignored`.

mod common;

use std::io::Read;
use std::process::{Command, Stdio};

use guixvis::model::IndexDoc;

use common::load_fixture;

#[test]
#[ignore = "requires `guix` on PATH and ~1-5 minutes"]
fn live_guix_indexer_produces_valid_index() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/data/guix-index.scm");
    let mut child = Command::new("guix")
        .args(["repl", "--", script, "testcommit", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn guix repl");
    let mut stdout = child.stdout.take().expect("stdout");
    let buf = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    })
    .join()
    .expect("stdout reader");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    let status = child.wait().expect("wait");
    assert!(status.success(), "guix repl failed: {stderr}");

    let doc: IndexDoc = serde_json::from_slice(&buf).expect("valid JSON");
    assert_eq!(doc.header.schema, 3);
    assert_eq!(doc.header.guix_commit, "testcommit");
    assert!(
        doc.header.package_count as usize >= 30000,
        "expected a full package set, got {}",
        doc.header.package_count
    );
    assert_eq!(doc.packages.len(), doc.header.package_count as usize);

    let index = guixvis::index::Index::from_doc(doc, 0).expect("index builds");
    assert!(index.names.contains_key("emacs"));
    assert!(index.names.contains_key("zlib"));
    assert!(index.names.contains_key("gtk+"));
    // Every package references a real name; spot-check known edges.
    let emacs = index.names["emacs"];
    let deps: Vec<&str> = index.packages[emacs as usize]
        .deps()
        .map(|(d, _)| index.packages[d as usize].name.as_ref())
        .collect();
    assert!(deps.contains(&"gtk+"), "emacs should depend on gtk+");
    let _ = load_fixture();
}

#[test]
#[ignore = "requires a built cache (~/.cache/guixvis) and a release build"]
fn real_index_search_latency() {
    let cache = guixvis::cache::Cache::new().expect("cache directory");
    let guixvis::cache::CacheStatus::Fresh(index) = cache.load(None, 0).expect("load snapshot")
    else {
        panic!("run guixvis once to build the cache");
    };
    let package_count = index.len();
    let index = std::sync::Arc::new(index);
    let worker = guixvis::search::SearchWorker::spawn(index);

    // Warm-up query (also builds the haystacks).
    worker.send("warmup".to_string());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while worker.take_reply(0).is_none() {
        assert!(std::time::Instant::now() < deadline, "no warm-up reply");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let t0 = std::time::Instant::now();
    worker.send("emac".to_string());
    let deadline = t0 + std::time::Duration::from_secs(10);
    while worker.take_reply(1).is_none() {
        assert!(std::time::Instant::now() < deadline, "no measured reply");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let dt = t0.elapsed();
    eprintln!(
        "search latency for 'emac' over {} real packages: {:.1} ms",
        package_count,
        dt.as_secs_f64() * 1000.0
    );
    assert!(
        dt.as_millis() < 100,
        "spec: first keystroke < 100 ms, got {} ms",
        dt.as_millis()
    );
}
