//! Graph view: budget caps, depth limits, and layout determinism.

mod common;

use guixvis::graph::{GraphView, NODE_BUDGET};

use common::load_fixture;

#[test]
fn projection_counts_nodes_beyond_the_drawing_budget() {
    let index = common::chain_fixture(8);
    let p = guixvis::graph::project(&index, 0, guixvis::graph::Dir::Deps, 7, 3);
    assert_eq!(p.nodes.len(), 3, "budget includes root");
    assert_eq!(p.truncated, 5, "discovery continues beyond materialization");
    assert_eq!(p.discovered_total, Some(8));
    assert!(p.discovery_complete);
    assert_eq!(p.edges_total, Some(2));
}

#[test]
fn dense_graph_reports_edge_budget_and_cancellation() {
    use guixvis::graph::{project_cancellable, Dir, EDGE_BUDGET};
    let count = 60u32;
    let packages:Vec<_>=(0..count).map(|id| serde_json::json!({"id":id,"name":format!("p{id}"),"inputs":(0..count).filter(|d| *d!=id).collect::<Vec<_>>()})).collect();
    let doc = serde_json::from_value(
        serde_json::json!({"header":{"schema":4,"package_count":count},"packages":packages}),
    )
    .unwrap();
    let index = guixvis::index::Index::from_doc(doc, 0).unwrap();
    let p = project_cancellable(&index, 0, Dir::Deps, 1, 200, &|| false).unwrap();
    assert_eq!(p.nodes.len(), 60);
    assert_eq!(p.edges.len(), EDGE_BUDGET);
    assert_eq!(p.edges_total, Some(3540));
    assert_eq!(p.edges_truncated, 540);
    assert!(matches!(
        project_cancellable(&index, 0, Dir::Deps, 1, 200, &|| true),
        Err(guixvis::relations::WalkError::Cancelled)
    ));
    assert!(matches!(
        project_cancellable(&index, 999, Dir::Deps, 1, 200, &|| false),
        Err(guixvis::relations::WalkError::InvalidRoot)
    ));
}

#[test]
fn graph_worker_discards_obsolete_depth_requests() {
    let worker =
        guixvis::graph_worker::GraphWorker::spawn(std::sync::Arc::new(common::chain_fixture(20)));
    worker.request(0, 8);
    let ticket = worker.request(0, 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let reply = loop {
        if let Some(reply) = worker.take_reply() {
            break reply;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(reply.ticket, ticket);
    assert_eq!(reply.view.depth, 1);
    assert_eq!(reply.view.nodes, vec![0, 1]);
    worker.cancel();
    assert!(worker.take_reply().is_none());
}

#[test]
fn discovery_and_edge_work_limits_report_unknown_totals_independently() {
    use guixvis::graph::{project, Dir};
    use guixvis::index::{Index, Package};
    use std::sync::Arc;
    // 199 * 10,051 outgoing relations exceed the 2M work cap. The root's
    // first hop is small, but counting edges among its selected nodes is not.
    let outside: Arc<[u32]> = (200..10_251).collect::<Vec<_>>().into();
    let direct: Arc<[u32]> = (1..200).collect::<Vec<_>>().into();
    let packages = (0..10_251)
        .map(|id| Package {
            id,
            catalog: true,
            name: format!("p{id}").into(),
            version: "1".into(),
            synopsis: "".into(),
            description: "".into(),
            homepage: "".into(),
            licenses: vec![].into(),
            file: "".into(),
            line: 0,
            inputs: if id == 0 {
                direct.clone()
            } else if id < 200 {
                outside.clone()
            } else {
                vec![].into()
            },
            propagated: vec![].into(),
            native: vec![].into(),
        })
        .collect();
    let index = Index::from_packages(packages, "fixture".into(), 0, 0).unwrap();
    let shallow = project(&index, 0, Dir::Deps, 1, 200);
    assert_eq!(shallow.discovered_total, Some(200));
    assert!(shallow.discovery_complete);
    assert_eq!(shallow.edges_total, None);
    assert_eq!(shallow.edges_truncated, 0);
    let deep = project(&index, 0, Dir::Deps, 2, 200);
    assert_eq!(deep.discovered_total, None);
    assert!(!deep.discovery_complete);
    assert_eq!(deep.nodes.len(), 200);
}

#[test]
fn builds_expected_subgraph() {
    let index = load_fixture();
    let mut g = GraphView::new(0, 2);
    g.rebuild(&index, 0, 2);
    // emacs -> gtk+, zlib, pkg-config, texinfo (5 nodes at depth 2).
    assert_eq!(g.nodes.len(), 5);
    assert!(g.nodes.contains(&index.names["gtk+"]));
    assert!(g.nodes.contains(&index.names["zlib"]));
    assert!(g.nodes.contains(&index.names["pkg-config"]));
    assert!(g.nodes.contains(&index.names["texinfo"]));
    assert!(g.laid_out);
    assert_eq!(g.pos.len(), 5);
}

#[test]
fn respects_node_budget_and_reports_truncation() {
    let index = load_fixture();
    let mut g = GraphView::new(0, 1);
    g.rebuild(&index, 0, 1);
    // emacs has 4 neighbors; the 200-node budget keeps them all.
    assert_eq!(g.nodes.len(), 5);
    assert!(g.nodes.len() <= NODE_BUDGET + 1, "root + budget");

    // The BFS cap itself truncates honestly: cap 2 materializes 2 of 4.
    let (bfs, total) = index.deps_bfs(0, 1, 2);
    assert_eq!(bfs.len(), 2);
    assert_eq!(total, 4, "discovered count stays honest under the cap");
}

#[test]
fn single_node_layout_is_stable() {
    let index = load_fixture();
    let mut g = GraphView::new(7, 2); // solo
    g.rebuild(&index, 7, 2);
    assert_eq!(g.nodes.len(), 1);
    assert_eq!(g.pos[0], (0.0, 0.0));
}

#[test]
fn layout_is_deterministic() {
    let index = load_fixture();
    let mut g1 = GraphView::new(0, 2);
    let mut g2 = GraphView::new(0, 2);
    g1.rebuild(&index, 0, 2);
    g2.rebuild(&index, 0, 2);
    assert_eq!(g1.pos.len(), g2.pos.len());
    for (a, b) in g1.pos.iter().zip(g2.pos.iter()) {
        assert!((a.0 - b.0).abs() < 1e-6, "x positions must match");
        assert!((a.1 - b.1).abs() < 1e-6, "y positions must match");
    }
}

#[test]
fn cycle_selection_wraps() {
    let index = load_fixture();
    let mut g = GraphView::new(0, 1);
    g.rebuild(&index, 0, 1);
    let n = g.nodes.len() as i32;
    g.selected = 0;
    g.select_delta(-1);
    assert_eq!(g.selected as i32, n - 1);
    g.select_delta(1);
    assert_eq!(g.selected, 0);
}

#[test]
fn depth_delta_clamps() {
    // The clamp lives in graph::clamp_depth and is what the app calls.
    assert_eq!(guixvis::graph::clamp_depth(1, -1), 1);
    assert_eq!(guixvis::graph::clamp_depth(8, 1), 8);
    assert_eq!(guixvis::graph::clamp_depth(3, 1), 4);
    assert_eq!(guixvis::graph::clamp_depth(3, -1), 2);
}
