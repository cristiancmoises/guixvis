//! Dependency graph view: budgeted BFS extraction + deterministic
//! Fruchterman–Reingold force-directed layout.
//!
//! The original spec named the `egraph` crate for layout, but the crate
//! published under that name (0.3.0) is an unrelated bioinformatics ML
//! binary. Layout is therefore hand-rolled: deterministic (seeded xorshift,
//! no external RNG), ~300 iterations, O(n²) per iteration — a few
//! milliseconds for the 200-node budget.

use std::collections::HashMap;

use crate::index::{DepKind, Index};

/// Maximum nodes materialized in the graph view.
pub const NODE_BUDGET: usize = 200;
/// Maximum edges materialized in the graph view.
pub const EDGE_BUDGET: usize = 3000;
/// Default BFS depth for the graph view.
pub const DEFAULT_DEPTH: u8 = 2;
/// Reverse-direction discovery cap (nodes considered before top-N selection).
const REVERSE_DISCOVERY_CAP: usize = 4000;

/// Which direction the graph projection follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Forward: the package's dependencies.
    Deps,
    /// Reverse: the package's dependents.
    Dependents,
}

/// A budgeted subgraph extracted from the index, shared by the TUI graph
/// view and the web API. `nodes[0]` is always the root.
#[derive(Debug)]
pub struct Projection {
    pub nodes: Vec<u32>,
    /// Edges as indices into `nodes`; every edge points from a dependent to
    /// its dependency (the "depends on" direction).
    pub edges: Vec<(u16, u16)>,
    /// Nodes discovered beyond the budget (not materialized). Never a total.
    pub truncated: usize,
    pub depth_of: Vec<u8>,
    pub kind_of: Vec<Option<DepKind>>,
}

/// Extract the budgeted graph for `root`.
///
/// `Dir::Deps` reproduces the TUI's forward BFS exactly (breadth-first
/// order, `NODE_BUDGET` cap, `EDGE_BUDGET` edge cap). `Dir::Dependents`
/// discovers reverse-reachable nodes depth-first-limited and materializes
/// the top `budget` by dependent count so high-degree hubs stay visible.
pub fn project(index: &Index, root: u32, dir: Dir, depth: u8, budget: usize) -> Projection {
    let budget = budget.min(NODE_BUDGET);
    match dir {
        Dir::Deps => {
            let (bfs, total) = index.deps_bfs(root, depth, budget);
            let mut nodes = Vec::with_capacity(bfs.len() + 1);
            nodes.push(root);
            let mut depth_of = Vec::with_capacity(bfs.len() + 1);
            depth_of.push(0);
            let mut kind_of = Vec::with_capacity(bfs.len() + 1);
            kind_of.push(None);
            for n in &bfs {
                nodes.push(n.id);
                depth_of.push(n.depth);
                kind_of.push(n.kind);
            }
            let edges = materialize_edges(index, &nodes);
            Projection {
                nodes,
                edges,
                truncated: total.saturating_sub(bfs.len()),
                depth_of,
                kind_of,
            }
        }
        Dir::Dependents => {
            let (bfs, total) = index.dependents_bfs(root, depth, REVERSE_DISCOVERY_CAP);
            let mut discovered: Vec<(u32, u8)> = bfs.iter().map(|n| (n.id, n.depth)).collect();
            // Prefer hubs: highest dependent count first, name as tie-break.
            discovered.sort_by(|a, b| {
                index
                    .dependents_count(b.0)
                    .cmp(&index.dependents_count(a.0))
                    .then_with(|| {
                        index.packages[a.0 as usize]
                            .name
                            .cmp(&index.packages[b.0 as usize].name)
                    })
            });
            let materialized = discovered.len().min(budget);
            discovered.truncate(budget);
            let mut nodes = Vec::with_capacity(discovered.len() + 1);
            nodes.push(root);
            let mut depth_of = Vec::with_capacity(discovered.len() + 1);
            depth_of.push(0);
            let mut kind_of = Vec::with_capacity(discovered.len() + 1);
            kind_of.push(None);
            for (id, d) in discovered {
                nodes.push(id);
                depth_of.push(d);
                kind_of.push(None);
            }
            let edges = materialize_edges(index, &nodes);
            Projection {
                nodes,
                edges,
                truncated: total.saturating_sub(materialized),
                depth_of,
                kind_of,
            }
        }
    }
}

/// Edges among materialized nodes (dependent -> dependency direction),
/// capped at [`EDGE_BUDGET`]. Uses an id→slot map instead of linear scans.
fn materialize_edges(index: &Index, nodes: &[u32]) -> Vec<(u16, u16)> {
    let slot: HashMap<u32, u16> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i as u16))
        .collect();
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for (idx, id) in nodes.iter().enumerate() {
        let p = &index.packages[*id as usize];
        for (dep, _) in p.deps() {
            if let Some(j) = slot.get(&dep) {
                if edges.len() < EDGE_BUDGET {
                    edges.push((idx as u16, *j));
                }
            }
        }
    }
    edges
}

pub struct GraphView {
    pub root: u32,
    pub depth: u8,
    /// Package ids of materialized nodes.
    pub nodes: Vec<u32>,
    /// Edges as indices into `nodes` (parent -> child, dependency direction).
    pub edges: Vec<(u16, u16)>,
    /// Laid-out positions in logical space (x ∈ [-1.6, 1.6], y ∈ [-1, 1]).
    pub pos: Vec<(f32, f32)>,
    pub selected: usize,
    /// Nodes discovered beyond the budget (honest truncation count).
    pub truncated: usize,
    pub laid_out: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        Self::new(0, DEFAULT_DEPTH)
    }
}

impl GraphView {
    pub fn new(root: u32, depth: u8) -> Self {
        GraphView {
            root,
            depth,
            nodes: Vec::new(),
            edges: Vec::new(),
            pos: Vec::new(),
            selected: 0,
            truncated: 0,
            laid_out: false,
        }
    }

    /// Rebuild the graph for `root` at `depth`, then layout.
    pub fn rebuild(&mut self, index: &Index, root: u32, depth: u8) {
        self.root = root;
        self.depth = depth;
        let projection = project(index, root, Dir::Deps, depth, NODE_BUDGET);
        self.nodes = projection.nodes;
        self.edges = projection.edges;
        self.truncated = projection.truncated;
        self.selected = 0;
        self.layout();
    }

    /// Deterministic Fruchterman–Reingold with linear cooling.
    pub fn layout(&mut self) {
        let n = self.nodes.len();
        self.pos = vec![(0.0, 0.0); n];
        if n == 0 {
            self.laid_out = true;
            return;
        }
        if n == 1 {
            self.pos[0] = (0.0, 0.0);
            self.laid_out = true;
            return;
        }

        // Initial placement on a circle with deterministic jitter.
        let mut rng = XorShift::new(0x9E37_79B9_7F4A_7C15);
        for (i, p) in self.pos.iter_mut().enumerate() {
            let angle = (i as f32) / (n as f32) * std::f32::consts::TAU;
            let jitter = rng.next_f32() * 0.1 - 0.05;
            *p = ((angle.cos() + jitter) * 0.9, (angle.sin() + jitter) * 0.9);
        }

        let area = 3.2 * 2.0;
        let k = (area / n as f32).sqrt().max(0.15);
        let iterations = 300;

        for iter in 0..iterations {
            let temp = 1.0 - (iter as f32 / iterations as f32);
            let temp = temp * temp * 1.2 + 0.02;
            let mut disp = vec![(0.0f32, 0.0f32); n];

            // Repulsion between every pair.
            for i in 0..n {
                for j in (i + 1)..n {
                    let (dx, dy) = (self.pos[i].0 - self.pos[j].0, self.pos[i].1 - self.pos[j].1);
                    let dist2 = dx * dx + dy * dy;
                    let dist = dist2.sqrt().max(0.05);
                    let force = k * k / dist;
                    let (fx, fy) = (force * dx / dist, force * dy / dist);
                    disp[i].0 += fx;
                    disp[i].1 += fy;
                    disp[j].0 -= fx;
                    disp[j].1 -= fy;
                }
            }

            // Attraction along edges.
            for (a, b) in &self.edges {
                let (dx, dy) = (
                    self.pos[*a as usize].0 - self.pos[*b as usize].0,
                    self.pos[*a as usize].1 - self.pos[*b as usize].1,
                );
                let dist = (dx * dx + dy * dy).sqrt().max(0.05);
                let force = dist * dist / k;
                let (fx, fy) = (force * dx / dist, force * dy / dist);
                disp[*a as usize].0 -= fx;
                disp[*a as usize].1 -= fy;
                disp[*b as usize].0 += fx;
                disp[*b as usize].1 += fy;
            }

            // Apply with temperature-limited displacement.
            for (d, p) in disp.iter().zip(self.pos.iter_mut()) {
                let (dx, dy) = *d;
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let capped = len.min(temp);
                p.0 += dx / len * capped;
                p.1 += dy / len * capped;
            }
        }

        // Normalize into the canvas space.
        self.fit();
        self.laid_out = true;
    }

    /// Scale positions into x ∈ [-1.6, 1.6], y ∈ [-1, 1] with margin.
    fn fit(&mut self) {
        let n = self.pos.len();
        if n == 0 {
            return;
        }
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &self.pos {
            min_x = min_x.min(p.0);
            max_x = max_x.max(p.0);
            min_y = min_y.min(p.1);
            max_y = max_y.max(p.1);
        }
        let span_x = (max_x - min_x).max(1e-6);
        let span_y = (max_y - min_y).max(1e-6);
        for p in &mut self.pos {
            p.0 = (p.0 - min_x) / span_x * 3.0 - 1.5;
            p.1 = (p.1 - min_y) / span_y * 1.8 - 0.9;
        }
    }

    /// Move the selection cursor to the next/previous node (wrapping).
    pub fn select_delta(&mut self, delta: i32) {
        if self.nodes.is_empty() {
            return;
        }
        let n = self.nodes.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(n)) as usize;
    }

    /// Selected package id, if any.
    pub fn selected_id(&self) -> Option<u32> {
        self.nodes.get(self.selected).copied()
    }
}

/// Clamp a depth delta against the valid graph depth range.
pub fn clamp_depth(cur: u8, delta: i8) -> u8 {
    (cur as i16 + delta as i16).clamp(1, 8) as u8
}

/// Minimal deterministic xorshift64* — no external RNG dependency.
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        XorShift(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}
