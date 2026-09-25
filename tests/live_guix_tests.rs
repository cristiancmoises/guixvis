//! Live integration: run the real Guile indexer against the host's Guix.
//! Ignored by default; run with `cargo test -- --ignored`.

mod common;

#[test]
#[ignore = "requires `guix` on PATH and ~1-5 minutes"]
fn live_guix_indexer_produces_valid_index() {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guix = guixvis::guix_env::current_guix().expect("resolve Guix");
    let origin = guixvis::guix_env::probe_origin(&guix, &cancel).expect("probe origin");
    let (tx, _rx) = std::sync::mpsc::channel();
    let started = std::time::Instant::now();
    let (doc, raw, commit) =
        guixvis::indexer::build(&guix, &origin, &cancel, &tx).expect("bounded extraction");
    eprintln!(
        "cold extraction: {:.3}s, {} bytes, {} objects ({} catalog), {} diagnostics, system {}",
        started.elapsed().as_secs_f64(),
        raw.len(),
        doc.packages.len(),
        doc.packages.iter().filter(|p| p.catalog).count(),
        doc.diagnostics.len(),
        origin.system
    );
    assert_eq!(doc.header.schema, guixvis::model::SCHEMA_VERSION);
    assert_eq!(doc.header.guix_commit, commit);
    assert_eq!(doc.header.origin, origin);
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
    // Exact object/category fidelity is independently checked by
    // tests/guix_indexer_live.scm; this checks the real Rust ingestion path.
    let emacs = index.names["emacs"];
    let deps: Vec<&str> = index.packages[emacs as usize]
        .deps()
        .map(|(d, _)| index.packages[d as usize].name.as_ref())
        .collect();
    assert!(deps.contains(&"gtk+"), "emacs should depend on gtk+");
    assert!(index.packages.iter().any(|p| !p.catalog));
    let blob = guixvis::blob::encode(&index);
    let started = std::time::Instant::now();
    let loaded = guixvis::blob::decode(&blob, 0).unwrap();
    assert_eq!(loaded.snapshot_id(), index.snapshot_id());
    eprintln!(
        "warm decode: {:.3}s, {} bytes",
        started.elapsed().as_secs_f64(),
        blob.len()
    );
}

#[test]
#[ignore = "requires a built cache (~/.cache/guixvis) and a release build"]
fn real_index_search_latency() {
    let cache = guixvis::cache::Cache::new().expect("cache directory");
    let (guixvis::cache::CacheStatus::Fresh(index)
    | guixvis::cache::CacheStatus::Unverified(index)) = cache.load(None, 0).expect("load snapshot")
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
