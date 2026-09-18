//! Fuzzy search ranking and highlight-range behavior through the real
//! SearchWorker path.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use guixvis::search::SearchWorker;

use common::load_fixture;

/// Wait up to 10s for a reply newer than `last`.
fn await_reply(worker: &SearchWorker, last: u64) -> guixvis::search::SearchReply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reply) = worker.take_reply(last) {
            return reply;
        }
        if Instant::now() > deadline {
            panic!("search worker did not reply in time");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn emac_ranks_emacs_first() {
    let index = Arc::new(load_fixture());
    let worker = SearchWorker::spawn(index);
    worker.send("emac".to_string());
    let reply = await_reply(&worker, 0);
    assert_eq!(reply.query, "emac");
    assert!(!reply.hits.is_empty());
    let fixture = load_fixture();
    let by_id: std::collections::HashMap<u32, &str> = fixture
        .packages
        .iter()
        .map(|p| (p.id, p.name.as_ref()))
        .collect();
    let top3: Vec<&str> = reply
        .hits
        .iter()
        .take(3)
        .map(|h| by_id[&h.hit.id])
        .collect();
    assert_eq!(top3[0], "emacs");
    assert!(top3.contains(&"emacs-minimal"));
    assert!(top3.contains(&"emacs-next"));
}

#[test]
fn empty_query_browses_alphabetically() {
    let index = Arc::new(load_fixture());
    let worker = SearchWorker::spawn(index);
    worker.send(String::new());
    let reply = await_reply(&worker, 0);
    assert_eq!(reply.hits.len(), 10);
    let index = load_fixture();
    let first = &index.packages[reply.hits[0].hit.id as usize].name;
    assert_eq!(first.as_ref(), "cyc-a");
}

#[test]
fn highlight_ranges_stay_within_name() {
    let index = Arc::new(load_fixture());
    let worker = SearchWorker::spawn(index);
    worker.send("emac".to_string());
    let reply = await_reply(&worker, 0);
    let emacs = reply.hits.iter().find(|h| {
        let index = load_fixture();
        index.packages[h.hit.id as usize].name.as_ref() == "emacs"
    });
    let emacs = emacs.expect("emacs among hits");
    assert!(!emacs.name_ranges.is_empty());
    for (s, e) in &emacs.name_ranges {
        assert!(*s < *e);
        assert!(*e <= 5, "name ranges must lie within \"emacs\" (5 chars)");
    }
}

#[test]
fn results_are_deterministic() {
    let index = Arc::new(load_fixture());
    let worker = SearchWorker::spawn(index);
    worker.send("z".to_string());
    let reply1 = await_reply(&worker, 0);
    worker.send("z".to_string());
    let reply2 = await_reply(&worker, 1);
    let ids1: Vec<u32> = reply1.hits.iter().map(|h| h.hit.id).collect();
    let ids2: Vec<u32> = reply2.hits.iter().map(|h| h.hit.id).collect();
    assert_eq!(ids1, ids2);
}

#[test]
fn stale_replies_are_dropped_by_ticket() {
    let index = Arc::new(load_fixture());
    let worker = SearchWorker::spawn(index);
    let t1 = worker.send("emac".to_string());
    worker.send("solo".to_string());
    let reply = await_reply(&worker, 0);
    // Latest-wins: the reply must carry ticket t1 + 1, not t1.
    assert!(reply.ticket > t1);
    assert_eq!(reply.query, "solo");
}
