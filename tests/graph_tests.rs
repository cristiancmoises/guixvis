//! Graph view: budget caps, depth limits, and layout determinism.

mod common;

use guixvis::graph::{GraphView, NODE_BUDGET};

use common::load_fixture;

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
