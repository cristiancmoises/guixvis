//! Dependency graph view: budgeted BFS extraction + deterministic
//! Fruchterman–Reingold force-directed layout.
//!
//! The original spec named the `egraph` crate for layout, but the crate
//! published under that name (0.3.0) is an unrelated bioinformatics ML
//! binary. Layout is therefore hand-rolled: deterministic (seeded xorshift,
//! no external RNG), ~300 iterations, O(n²) per iteration — a few
//! milliseconds for the 200-node budget.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::index::{DepKind, Index};

/// Maximum nodes materialized in the graph view.
pub const NODE_BUDGET: usize = 200;
/// Maximum edges materialized in the graph view.
pub const EDGE_BUDGET: usize = 3000;
/// Default BFS depth for the graph view.
pub const DEFAULT_DEPTH: u8 = 2;
/// The terminal begins with direct dependencies only; web keeps depth two.
pub const TUI_DEFAULT_DEPTH: u8 = 1;
const DISCOVERY_WORK_BUDGET: usize = 2_000_000;

/// How much of the edge set the terminal graph draws.
///
/// A 200-node graph carries hundreds of edges; drawn at full strength they
/// bury the nodes they are supposed to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeMode {
    /// Every edge, faded well into the background.
    All,
    /// Only the edges the selected node touches.
    #[default]
    Focus,
    /// No edges at all.
    None,
}

impl EdgeMode {
    pub fn next(self) -> Self {
        match self {
            EdgeMode::All => EdgeMode::Focus,
            EdgeMode::Focus => EdgeMode::None,
            EdgeMode::None => EdgeMode::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EdgeMode::All => "all edges",
            EdgeMode::Focus => "edges at selection",
            EdgeMode::None => "no edges",
        }
    }
}

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
#[derive(Debug, Default)]
pub struct Projection {
    pub nodes: Vec<u32>,
    /// Edges as indices into `nodes`; every edge points from a dependent to
    /// its dependency (the "depends on" direction).
    pub edges: Vec<(u16, u16)>,
    /// Nodes discovered beyond the budget (not materialized). Never a total.
    pub truncated: usize,
    pub depth_of: Vec<u8>,
    pub kind_of: Vec<Option<DepKind>>,
    /// Union of relationship categories encountered while discovering each node.
    pub kinds_of: Vec<Vec<DepKind>>,
    pub discovered_total: Option<usize>,
    pub edges_total: Option<usize>,
    pub edges_truncated: usize,
    pub discovery_complete: bool,
}

/// Synchronous projection used by HTTP and tests. The budget includes root.
pub fn project(index: &Index, root: u32, dir: Dir, depth: u8, budget: usize) -> Projection {
    project_cancellable(index, root, dir, depth, budget, &|| false).unwrap_or_default()
}

pub fn project_cancellable(
    index: &Index,
    root: u32,
    dir: Dir,
    depth: u8,
    budget: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Projection, crate::relations::WalkError> {
    use crate::relations::WalkError;
    if root as usize >= index.len() {
        return Err(WalkError::InvalidRoot);
    }
    let mut seen = vec![false; index.len()];
    let mut kinds = vec![0u8; index.len()];
    let mut queue = VecDeque::from([(root, 0u8)]);
    let mut reached = vec![(root, 0u8, None)];
    seen[root as usize] = true;
    let mut work = 0usize;
    let mut complete = true;
    'discovery: while let Some((id, d)) = queue.pop_front() {
        if cancelled() {
            return Err(WalkError::Cancelled);
        }
        if d >= depth {
            continue;
        }
        let neighbors: Box<dyn Iterator<Item = (u32, Option<DepKind>)> + '_> = match dir {
            Dir::Deps => Box::new(
                index.packages[id as usize]
                    .typed_deps()
                    .map(|(id, k)| (id, Some(k))),
            ),
            Dir::Dependents => Box::new(
                index.dependents[id as usize]
                    .iter()
                    .copied()
                    .map(|id| (id, None)),
            ),
        };
        for (dep, kind) in neighbors {
            if work >= DISCOVERY_WORK_BUDGET {
                complete = false;
                break 'discovery;
            }
            work += 1;
            let categories = match dir {
                Dir::Deps => kind.into_iter().collect::<Vec<_>>(),
                Dir::Dependents => index.packages[dep as usize].dep_kinds(id),
            };
            for category in categories {
                kinds[dep as usize] |= match category {
                    DepKind::Input => 1,
                    DepKind::Propagated => 2,
                    DepKind::Native => 4,
                };
            }
            if work.is_multiple_of(256) && cancelled() {
                return Err(WalkError::Cancelled);
            }
            if !seen[dep as usize] {
                seen[dep as usize] = true;
                reached.push((dep, d + 1, kind));
                queue.push_back((dep, d + 1));
            }
        }
    }
    let discovered = reached.len();
    if dir == Dir::Dependents {
        reached[1..].sort_unstable_by(|a, b| {
            index
                .dependents_count(b.0)
                .cmp(&index.dependents_count(a.0))
                .then_with(|| {
                    index.packages[a.0 as usize]
                        .name
                        .cmp(&index.packages[b.0 as usize].name)
                })
                .then(a.0.cmp(&b.0))
        });
    }
    reached.truncate(budget.min(NODE_BUDGET));
    let nodes: Vec<_> = reached.iter().map(|n| n.0).collect();
    let slots: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i as u16))
        .collect();
    let mut edges = Vec::new();
    let mut total_edges = 0usize;
    let mut edge_work = 0usize;
    let mut edges_complete = true;
    'edges: for (i, &id) in nodes.iter().enumerate() {
        let mut targets = HashSet::new();
        for (dep, _) in index.packages[id as usize].typed_deps() {
            if edge_work >= DISCOVERY_WORK_BUDGET {
                edges_complete = false;
                break 'edges;
            }
            edge_work += 1;
            if edge_work.is_multiple_of(256) && cancelled() {
                return Err(WalkError::Cancelled);
            }
            if let Some(&j) = slots.get(&dep) {
                if targets.insert(j) {
                    total_edges += 1;
                    if edges.len() < EDGE_BUDGET {
                        edges.push((i as u16, j));
                    }
                }
            }
        }
    }
    if cancelled() {
        return Err(WalkError::Cancelled);
    }
    Ok(Projection {
        truncated: discovered.saturating_sub(nodes.len()),
        discovered_total: complete.then_some(discovered),
        discovery_complete: complete,
        edges_truncated: total_edges.saturating_sub(edges.len()),
        edges_total: edges_complete.then_some(total_edges),
        nodes,
        edges,
        depth_of: reached.iter().map(|n| n.1).collect(),
        kind_of: reached.iter().map(|n| n.2).collect(),
        kinds_of: reached
            .iter()
            .map(|n| {
                [
                    (1, DepKind::Input),
                    (2, DepKind::Propagated),
                    (4, DepKind::Native),
                ]
                .into_iter()
                .filter_map(|(bit, kind)| (kinds[n.0 as usize] & bit != 0).then_some(kind))
                .collect()
            })
            .collect(),
    })
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
    /// BFS depth of each node (0 for the root); drives the color ramp.
    pub depth_of: Vec<u8>,
    /// Edge kind each node was reached through (None for the root); drives
    /// the edge coloring.
    pub kind_of: Vec<Option<DepKind>>,
    pub kinds_of: Vec<Vec<DepKind>>,
    /// How long the last layout took, for the status line.
    pub layout_ms: f64,
    pub discovered_total: Option<usize>,
    pub edges_total: Option<usize>,
    pub edges_truncated: usize,
    pub discovery_complete: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        Self::new(0, TUI_DEFAULT_DEPTH)
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
            depth_of: Vec::new(),
            kind_of: Vec::new(),
            kinds_of: Vec::new(),
            layout_ms: 0.0,
            discovered_total: None,
            edges_total: None,
            edges_truncated: 0,
            discovery_complete: false,
        }
    }

    /// Rebuild the graph for `root` at `depth`, then layout.
    pub fn rebuild(&mut self, index: &Index, root: u32, depth: u8) {
        self.rebuild_dir(index, root, depth, Dir::Deps);
    }

    /// Rebuild for an explicit direction (forward dependencies or reverse
    /// dependents), then layout.
    pub fn rebuild_dir(&mut self, index: &Index, root: u32, depth: u8, dir: Dir) {
        if let Ok(view) = Self::build_cancellable(index, root, depth, dir, &|| false) {
            *self = view;
        }
    }

    pub fn build_cancellable(
        index: &Index,
        root: u32,
        depth: u8,
        dir: Dir,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, crate::relations::WalkError> {
        let projection = project_cancellable(index, root, dir, depth, NODE_BUDGET, cancelled)?;
        let mut view = Self::new(root, depth);
        view.apply_projection(root, depth, projection);
        let started = std::time::Instant::now();
        view.layout_cancellable(cancelled)?;
        view.layout_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(view)
    }

    fn apply_projection(&mut self, root: u32, depth: u8, projection: Projection) {
        self.root = root;
        self.depth = depth;
        self.nodes = projection.nodes;
        self.edges = projection.edges;
        self.truncated = projection.truncated;
        self.depth_of = projection.depth_of;
        self.kind_of = projection.kind_of;
        self.kinds_of = projection.kinds_of;
        self.discovered_total = projection.discovered_total;
        self.discovery_complete = projection.discovery_complete;
        self.edges_total = projection.edges_total;
        self.edges_truncated = projection.edges_truncated;
        self.selected = 0;
    }

    /// Deterministic Fruchterman–Reingold with linear cooling.
    pub fn layout(&mut self) {
        let _ = self.layout_cancellable(&|| false);
    }

    fn layout_cancellable(
        &mut self,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), crate::relations::WalkError> {
        if cancelled() {
            return Err(crate::relations::WalkError::Cancelled);
        }
        let n = self.nodes.len();
        self.pos = vec![(0.0, 0.0); n];
        if n == 0 {
            self.laid_out = true;
            return Ok(());
        }
        if n == 1 {
            self.pos[0] = (0.0, 0.0);
            self.laid_out = true;
            return Ok(());
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

        // NOTE: a uniform-grid approximation of the repulsion sum was tried
        // here and measured slower than the exact pairwise loop at the
        // 200-node cap (12.4 ms vs 9.9 ms for emacs at depth 2), so the simple
        // version stayed. Run `cargo run --release --example bench` before
        // reaching for a fancier layout.
        for iter in 0..iterations {
            if cancelled() {
                return Err(crate::relations::WalkError::Cancelled);
            }
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
        Ok(())
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
