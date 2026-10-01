mod common;
use guixvis::relations::{collect_relations, matches_relation, Direction, WalkError};

#[test]
fn reverse_diamond_cycle_preserves_order_shortest_depth_and_all_categories() {
    use guixvis::index::{
        DepKind::{Input, Native, Propagated},
        Index,
    };
    let doc = serde_json::from_value(serde_json::json!({
        "header": {"schema": 4, "package_count": 4}, "packages": [
            {"id": 0, "name": "root", "inputs": [3]},
            {"id": 1, "name": "zeta", "inputs": [0], "propagated_inputs": [0], "native_inputs": [0]},
            {"id": 2, "name": "alpha", "propagated_inputs": [0]},
            {"id": 3, "name": "middle", "inputs": [1], "propagated_inputs": [1], "native_inputs": [2]}
        ]
    })).unwrap();
    let index = Index::from_doc(doc, 0).unwrap();
    let set = collect_relations(&index, 0, Direction::Dependents, &|| false).unwrap();
    assert_eq!(
        set.hits
            .iter()
            .map(|h| (h.id, h.depth, h.kinds.clone()))
            .collect::<Vec<_>>(),
        vec![
            (2, 1, vec![Propagated]),
            (3, 2, vec![Input, Propagated, Native]),
            (1, 1, vec![Input, Propagated, Native])
        ]
    );
    assert_eq!(set.key.snapshot, index.snapshot_id());
}

#[test]
fn literal_unicode_relation_filters_preserve_terms_and_field_case_folding() {
    for (name, version, query, want) in [
        ("Éditeur", "ΒΣ", "\u{2003}ÉDI\tβς", true),
        ("ΟΣ", "İ2", "ος i\u{307}", true),
        ("Éditeur", "3.2", "ÉDI\n3.2", true),
        ("Éditeur", "3.2", ".*", false),
        ("a[b]", "1", "[b]", true),
        ("Éditeur", "3.2", "\u{2003}\t", true),
        ("Éditeur", "3.2", "édi 4", false),
    ] {
        assert_eq!(
            matches_relation(name, version, query),
            want,
            "{name} {version}: {query:?}"
        );
    }
}

#[test]
fn full_closures_reach_beyond_depth_255_and_two_thousand_rows() {
    let index = common::chain_fixture(2502);
    for (root, direction, last) in [
        (0, Direction::Dependencies, 2501),
        (2501, Direction::Dependents, 0),
    ] {
        let set = collect_relations(&index, root, direction, &|| false).unwrap();
        assert_eq!(set.hits.len(), 2501);
        assert_eq!(
            set.hits.iter().find(|hit| hit.id == last).unwrap().depth,
            2501
        );
    }
    assert!(matches_relation("p02501", "1", "P02501 1"));
    assert!(!matches_relation("p02501", "1", "P02501 9"));
    assert!(matches_relation("Éditeur", "3.2", "ÉDI 3.2"));
    assert!(!matches_relation("Éditeur", "3.2", ".*"));
}

#[test]
fn cycles_kinds_invalid_roots_and_cancellation() {
    let index = common::identity_fixture();
    let set = collect_relations(&index, 0, Direction::Dependencies, &|| false).unwrap();
    assert_eq!(set.hits.len(), 5);
    assert!(!set.hits.iter().any(|hit| hit.id == 0));
    assert_eq!(
        set.hits.iter().find(|hit| hit.id == 1).unwrap().kinds.len(),
        3
    );
    assert_eq!(
        collect_relations(&index, 0, Direction::Dependencies, &|| true).unwrap_err(),
        WalkError::Cancelled
    );
    assert_eq!(
        collect_relations(&index, 999, Direction::Dependencies, &|| false).unwrap_err(),
        WalkError::InvalidRoot
    );
}

#[test]
fn diamond_unions_kinds_and_uses_shortest_distance() {
    let doc = serde_json::from_value(serde_json::json!({
        "header": {"schema":4, "package_count":4}, "packages": [
            {"id":0,"name":"root","inputs":[1,2]}, {"id":1,"name":"a","inputs":[3]},
            {"id":2,"name":"b","native_inputs":[3]}, {"id":3,"name":"c","inputs":[0]}
        ]
    }))
    .unwrap();
    let index = guixvis::index::Index::from_doc(doc, 0).unwrap();
    let set = collect_relations(&index, 0, Direction::Dependencies, &|| false).unwrap();
    let c = set.hits.iter().find(|h| h.id == 3).unwrap();
    assert_eq!(c.depth, 2);
    assert_eq!(
        c.kinds,
        vec![
            guixvis::index::DepKind::Input,
            guixvis::index::DepKind::Native
        ]
    );
    let singleton = common::chain_fixture(1);
    assert!(
        collect_relations(&singleton, 0, Direction::Dependencies, &|| false)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn worker_reuses_closure_and_only_publishes_current_ticket() {
    use guixvis::relations::{RelationReply, RelationWorker};
    use std::sync::Arc;
    let worker = RelationWorker::spawn(Arc::new(common::chain_fixture(2502)));
    fn wait(worker: &RelationWorker) -> RelationReply {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(reply) = worker.take_reply() {
                return reply;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    worker.request(0, Direction::Dependencies, String::new());
    let all = wait(&worker);
    assert_eq!(all.visible.len(), 2501);
    worker.request(0, Direction::Dependencies, "obsolete".into());
    let ticket = worker.request(0, Direction::Dependencies, "p02501".into());
    let filtered = wait(&worker);
    assert_eq!(filtered.ticket, ticket);
    assert_eq!(filtered.query, "p02501");
    assert_eq!(filtered.visible.len(), 1);
    assert!(
        Arc::ptr_eq(&all.set, &filtered.set),
        "filter must reuse the computed closure"
    );
    worker.request(2501, Direction::Dependents, "p00000".into());
    let reverse = wait(&worker);
    assert_eq!(reverse.key.root, 2501);
    assert_eq!(reverse.set.hits[reverse.visible[0]].id, 0);
    worker.cancel();
    assert!(worker.take_reply().is_none());
}
