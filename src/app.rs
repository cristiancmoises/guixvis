//! Application state, phase management, and keyboard dispatch.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::graph::{EdgeMode, GraphView, TUI_DEFAULT_DEPTH};
use crate::graph_worker::GraphWorker;
use crate::index::Index;
use crate::indexer::{self, IndexEvent};
use crate::relations::{Direction, RelationSet, RelationWorker};
use crate::search::{HighlightedHit, SearchWorker, DEBOUNCE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Navigate,
    Search,
}

pub enum QueryEdit {
    Push(char),
    Backspace,
    Clear,
}

#[derive(Debug, Clone)]
pub struct GraphContext {
    pub root: u32,
    pub depth: u8,
    pub selected_id: Option<u32>,
    pub query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Deps,
    RevDeps,
    Graph,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Overview, Tab::Deps, Tab::RevDeps, Tab::Graph];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Deps => "Dependencies",
            Tab::RevDeps => "Reverse deps",
            Tab::Graph => "Graph",
        }
    }

    pub fn key(self) -> char {
        match self {
            Tab::Overview => '1',
            Tab::Deps => '2',
            Tab::RevDeps => '3',
            Tab::Graph => '4',
        }
    }

    fn next(self) -> Tab {
        Tab::ALL[(self as usize + 1) % Tab::ALL.len()]
    }

    fn prev(self) -> Tab {
        Tab::ALL[(self as usize + Tab::ALL.len() - 1) % Tab::ALL.len()]
    }
}

/// Edge-kind tag used in tree node keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Input,
    Propagated,
    Native,
    RevDirect,
    RevTrans,
}

pub type NodeKey = Vec<(u32, NodeKind)>;

/// Channel metadata may contain arbitrary URI schemes; only open web pages.
fn is_web_homepage(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && !rest.starts_with('/')
        && !rest.starts_with('?')
        && !rest.starts_with('#')
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[derive(Debug, Default)]
pub struct TreeState {
    /// Keys of nodes whose children are expanded.
    pub expanded: HashSet<NodeKey>,
    pub cursor: usize,
    pub scroll: usize,
    pub query: String,
    pub root: Option<u32>,
    pub relations: Option<Arc<RelationSet>>,
    pub visible: Vec<usize>,
    pub rows: Arc<crate::ui::tree::TreeRows>,
}

#[derive(Debug, Default)]
pub struct RevState {
    pub expanded: HashSet<NodeKey>,
    /// Whether the "Transitive" section is expanded.
    pub trans_open: bool,
    /// Local filter for dependents of the selected Overview package.
    pub query: String,
    pub root: Option<u32>,
    pub cursor: usize,
    pub scroll: usize,
    pub relations: Option<Arc<RelationSet>>,
    pub visible: Vec<usize>,
    pub rows: Arc<crate::ui::tree::TreeRows>,
}

#[derive(Debug)]
pub enum Phase {
    Loading { done: u64, total: u64 },
    Ready { fresh: bool, unkeyed: bool },
    Failed { msg: String },
}

pub struct App {
    pub index: Option<Arc<Index>>,
    pub phase: Phase,
    pub query: String,
    pending_query: Option<String>,
    last_edit: Instant,
    pub results: Vec<HighlightedHit>,
    rendered_ticket: u64,
    requested_ticket: u64,
    pub cursor: usize,
    pub scroll: usize,
    pub tab: Tab,
    pub mode: InputMode,
    pub anchor: Option<u32>,
    pub graph_query: String,
    pub graph_history: Vec<GraphContext>,
    pub graph_visible: Vec<usize>,
    pub graph_scroll: usize,
    pub graph_details_scroll: u16,
    graph_worker: Option<GraphWorker>,
    graph_ticket: u64,
    graph_requested: Option<(u32, u8)>,
    graph_pending: bool,
    graph_restore_selection: Option<u32>,
    relations: Option<RelationWorker>,
    relation_ticket: u64,
    relations_pending: bool,
    pub tree: TreeState,
    pub rev: RevState,
    pub graph: GraphView,
    graph_anchor: Option<u32>,
    pub theme_idx: usize,
    pub help_open: bool,
    pub dirty: bool,
    pub tick: u64,
    pub size: (u16, u16),
    pub quit: bool,
    pub search: Option<SearchWorker>,
    events: Option<Receiver<IndexEvent>>,
    cancel: Arc<AtomicBool>,
    loader: Option<JoinHandle<()>>,
    /// Whether the last user action was a graph rebuild (drives dirty flag).
    pub graph_dirty: bool,
    /// How much of the edge set the graph tab draws.
    pub edge_mode: EdgeMode,
    /// Whether hub labels are drawn in the graph tab.
    pub graph_labels: bool,
}

impl App {
    /// Create the app and start the cache/index loader in the background.
    pub fn new(force_rebuild: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let loader = indexer::start_loader(tx, Arc::clone(&cancel), force_rebuild);
        App {
            index: None,
            phase: Phase::Loading { done: 0, total: 0 },
            query: String::new(),
            pending_query: None,
            last_edit: Instant::now(),
            results: Vec::new(),
            rendered_ticket: 0,
            requested_ticket: 0,
            cursor: 0,
            scroll: 0,
            tab: Tab::Overview,
            mode: InputMode::Search,
            anchor: None,
            graph_query: String::new(),
            graph_history: Vec::new(),
            graph_visible: Vec::new(),
            graph_scroll: 0,
            graph_details_scroll: 0,
            graph_worker: None,
            graph_ticket: 0,
            graph_requested: None,
            graph_pending: false,
            graph_restore_selection: None,
            relations: None,
            relation_ticket: 0,
            relations_pending: false,
            tree: TreeState::default(),
            rev: RevState::default(),
            graph: GraphView::default(),
            graph_anchor: None,
            theme_idx: crate::theme::saved_theme(),
            help_open: false,
            dirty: true,
            tick: 0,
            size: (80, 24),
            quit: false,
            search: None,
            events: Some(rx),
            cancel,
            loader: Some(loader),
            graph_dirty: true,
            edge_mode: EdgeMode::default(),
            graph_labels: true,
        }
    }

    pub fn selected_id(&self) -> Option<u32> {
        if self.tab == Tab::Overview {
            self.results.get(self.cursor).map(|h| h.hit.id)
        } else {
            self.anchor
        }
    }

    pub fn selected_pkg(&self) -> Option<&crate::index::Package> {
        let id = self.selected_id()?;
        self.index.as_ref()?.packages.get(id as usize)
    }

    pub fn rev_root_id(&self) -> Option<u32> {
        self.rev.root.or_else(|| self.selected_id())
    }

    pub fn rev_root_pkg(&self) -> Option<&crate::index::Package> {
        self.index
            .as_ref()?
            .packages
            .get(self.rev_root_id()? as usize)
    }

    pub fn animating(&self) -> bool {
        matches!(self.phase, Phase::Loading { .. })
    }

    /// Keep input and reply polling responsive until the current query settles.
    pub fn search_pending(&self) -> bool {
        self.pending_query.is_some()
            || self.requested_ticket > self.rendered_ticket
            || self.relations_pending
            || self.graph_pending
    }

    pub fn relations_pending(&self) -> bool {
        self.relations_pending
    }

    pub fn graph_pending(&self) -> bool {
        self.graph_pending
    }

    /// Pump loader/indexer events. Returns true if something changed.
    pub fn pump_events(&mut self) -> bool {
        let Some(events) = self.events.take() else {
            return false;
        };
        let mut changed = false;
        while let Ok(ev) = events.try_recv() {
            changed = true;
            match ev {
                IndexEvent::Progress { done, total } => {
                    self.phase = Phase::Loading { done, total };
                }
                IndexEvent::Ready {
                    index,
                    fresh,
                    unkeyed,
                } => {
                    self.attach_index(index, fresh, unkeyed);
                }
                IndexEvent::Failed { msg } => {
                    self.phase = Phase::Failed { msg };
                }
            }
        }
        self.events = Some(events);
        changed
    }

    fn attach_index(&mut self, index: Arc<Index>, fresh: bool, unkeyed: bool) {
        // (Re)create the search worker for the new index.
        self.search = Some(SearchWorker::spawn(Arc::clone(&index)));
        self.relations = Some(RelationWorker::spawn(Arc::clone(&index)));
        self.graph_worker = Some(GraphWorker::spawn(Arc::clone(&index)));
        self.index = Some(index);
        self.phase = Phase::Ready { fresh, unkeyed };
        self.results.clear();
        self.rendered_ticket = 0;
        self.cursor = 0;
        self.scroll = 0;
        self.tree.expanded.clear();
        self.tree.cursor = 0;
        self.tree.scroll = 0;
        self.rev.expanded.clear();
        self.rev.trans_open = false;
        self.rev.query.clear();
        self.rev.root = None;
        self.rev.cursor = 0;
        self.rev.scroll = 0;
        self.graph = GraphView::default();
        self.tree = TreeState::default();
        self.rev = RevState::default();
        self.anchor = None;
        self.graph_history.clear();
        self.graph_requested = None;
        self.graph_pending = false;
        self.graph_restore_selection = None;
        self.graph_visible.clear();
        self.graph_scroll = 0;
        self.graph_query.clear();
        self.relations_pending = false;
        self.tab = Tab::Overview;
        self.mode = InputMode::Search;
        self.graph_anchor = None;
        self.graph_dirty = true;
        // Re-dispatch any pending query against the fresh index.
        self.dispatch_query(self.query.clone());
    }

    /// Check the search mailbox and adopt newer replies.
    pub fn pump_search(&mut self) -> bool {
        let Some(search) = self.search.as_ref() else {
            return false;
        };
        let Some(reply) = search.take_reply(self.rendered_ticket) else {
            return false;
        };
        self.rendered_ticket = reply.ticket;
        if self.tab != Tab::Overview
            || reply.ticket != self.requested_ticket
            || reply.query != self.query
        {
            return false;
        }
        let selected = self.selected_id();
        self.results = reply.hits;
        self.cursor = self.cursor.min(self.results.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.results.len().saturating_sub(1));
        if selected != self.selected_id() {
            if self.tab == Tab::RevDeps {
                self.tree.cursor = 0;
                self.tree.scroll = 0;
            } else {
                self.reset_tree_positions();
            }
        }
        if self.tab == Tab::Graph {
            self.ensure_graph();
        }
        true
    }

    /// Per-tick work: flush debounced queries.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.pump_relations();
        self.pump_graph();
        if let Some(q) = self.pending_query.take() {
            if self.last_edit.elapsed() >= DEBOUNCE {
                self.dispatch_query(q);
            } else {
                self.pending_query = Some(q);
            }
        }
    }

    fn dispatch_query(&mut self, query: String) {
        if let Some(search) = self.search.as_ref() {
            self.requested_ticket = search.send(query);
        }
    }

    /// Rebuild the index in the background (keeps current index usable).
    pub fn rebuild(&mut self) {
        if matches!(self.phase, Phase::Loading { .. }) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.events = Some(rx);
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        let loader = indexer::start_loader(tx, Arc::clone(&self.cancel), true);
        self.loader = Some(loader);
        self.phase = Phase::Loading { done: 0, total: 0 };
        self.dirty = true;
    }

    /// Open the selected package's homepage in the system browser.
    pub fn open_homepage(&self) {
        let Some(pkg) = self.selected_pkg() else {
            return;
        };
        if !is_web_homepage(&pkg.homepage) {
            return;
        }
        let url = pkg.homepage.as_ref();
        let mut cmd = std::env::var_os("BROWSER")
            .map(|b| {
                let mut c = std::process::Command::new(b);
                c.arg(url);
                c
            })
            .unwrap_or_else(|| {
                let mut c = std::process::Command::new("xdg-open");
                c.arg(url);
                c
            });
        let _ = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    fn switch_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        if self.tab == Tab::Overview {
            let root = self.selected_id();
            if self.anchor != root {
                self.tree = TreeState::default();
                self.rev = RevState::default();
                self.graph = GraphView::default();
                self.graph_anchor = None;
                self.graph_requested = None;
                self.graph_visible.clear();
                self.graph_pending = false;
                if let Some(worker) = &self.graph_worker {
                    worker.cancel();
                }
                self.graph_query.clear();
                self.graph_history.clear();
            }
            self.anchor = root;
            self.tree.root = root;
            self.rev.root = root;
            self.pending_query = None;
            if let Some(search) = &self.search {
                let ticket = search.invalidate();
                self.requested_ticket = ticket;
                self.rendered_ticket = ticket;
            }
        }
        self.tab = tab;
        self.mode = InputMode::Navigate;
        if tab == Tab::Overview {
            self.dispatch_query(self.query.clone());
        }
        self.request_relations();
        if tab == Tab::Graph {
            self.ensure_graph();
        }
        self.dirty = true;
    }

    /// (Re)build the graph view from the current selection.
    pub fn ensure_graph(&mut self) {
        let Some(id) = self.selected_id() else {
            if self.graph_anchor.is_some() || !self.graph.nodes.is_empty() {
                self.graph = GraphView::default();
                self.graph_anchor = None;
                self.graph_requested = None;
                self.graph_visible.clear();
                self.graph_pending = false;
                if let Some(worker) = &self.graph_worker {
                    worker.cancel();
                }
                self.dirty = true;
            }
            return;
        };
        if self.graph_anchor != Some(id) || self.graph_requested.is_none() {
            self.graph_anchor = Some(id);
            self.request_graph(id, TUI_DEFAULT_DEPTH, None);
            self.dirty = true;
        }
    }

    /// Toggle expansion of the tree row under the cursor (Deps/RevDeps tabs).
    pub fn toggle_row(&mut self, key: NodeKey) {
        match self.tab {
            Tab::Deps => {
                if self.tree.expanded.contains(&key) {
                    self.tree.expanded.remove(&key);
                } else {
                    self.tree.expanded.insert(key);
                }
            }
            Tab::RevDeps => {
                // u32::MAX is the sentinel id of the "Transitive" section row.
                if key.last() == Some(&(u32::MAX, NodeKind::RevTrans)) {
                    self.rev.trans_open = !self.rev.trans_open;
                } else if self.rev.expanded.contains(&key) {
                    self.rev.expanded.remove(&key);
                } else {
                    self.rev.expanded.insert(key);
                }
            }
            _ => return,
        }
        self.refresh_tree_rows();
        self.dirty = true;
    }

    /// Follow the selected graph node as the new root.
    pub fn graph_follow(&mut self) {
        let Some(id) = self.graph.selected_id() else {
            return;
        };
        if id == self.graph.root {
            return;
        }
        self.graph_history.push(GraphContext {
            root: self.graph.root,
            depth: self.graph.depth,
            selected_id: Some(id),
            query: self.graph_query.clone(),
        });
        self.graph_query.clear();
        self.request_graph(id, self.graph.depth, None);
        self.dirty = true;
    }

    pub fn graph_depth_delta(&mut self, delta: i8) {
        let depth = crate::graph::clamp_depth(self.graph.depth, delta);
        if depth != self.graph.depth {
            self.request_graph(self.graph.root, depth, self.graph.selected_id());
            self.dirty = true;
        }
    }

    /// Move the list cursor, clamping scroll to keep it visible.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.row_count() as i32;
        if len == 0 {
            return;
        }
        let cursor = (self.current_cursor() as i32 + delta).clamp(0, len - 1) as usize;
        self.set_current_cursor(cursor);
        self.dirty = true;
    }

    pub fn page_cursor(&mut self, page: usize, down: bool) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        let cursor = if down {
            self.current_cursor().saturating_add(page).min(len - 1)
        } else {
            self.current_cursor().saturating_sub(page)
        };
        self.set_current_cursor(cursor);
        self.dirty = true;
    }

    pub fn jump_top(&mut self) {
        self.set_current_cursor(0);
        self.dirty = true;
    }

    pub fn jump_bottom(&mut self) {
        let len = self.row_count();
        if len > 0 {
            self.set_current_cursor(len - 1);
            self.dirty = true;
        }
    }

    /// Number of navigable rows in the current tab.
    pub fn row_count(&self) -> usize {
        match self.tab {
            Tab::Overview => self.results.len(),
            Tab::Deps => self.tree.rows.rows.len(),
            Tab::RevDeps => self.rev.rows.rows.len(),
            Tab::Graph => self.graph_visible.len(),
        }
    }

    fn current_cursor(&self) -> usize {
        match self.tab {
            Tab::Overview => self.cursor,
            Tab::Deps => self.tree.cursor,
            Tab::RevDeps => self.rev.cursor,
            Tab::Graph => self
                .graph_visible
                .iter()
                .position(|i| *i == self.graph.selected)
                .unwrap_or(0),
        }
    }

    fn set_current_cursor(&mut self, cursor: usize) {
        match self.tab {
            Tab::Overview => {
                if self.cursor != cursor {
                    self.cursor = cursor;
                    self.reset_tree_positions();
                }
            }
            Tab::Deps => self.tree.cursor = cursor,
            Tab::RevDeps => self.rev.cursor = cursor,
            Tab::Graph => {
                let selected = self
                    .graph_visible
                    .get(cursor)
                    .copied()
                    .unwrap_or(usize::MAX);
                if self.graph.selected != selected {
                    self.graph.selected = selected;
                    self.graph_details_scroll = 0;
                }
            }
        }
    }

    fn reset_tree_positions(&mut self) {
        self.tree.cursor = 0;
        self.tree.scroll = 0;
        self.rev.cursor = 0;
        self.rev.scroll = 0;
    }

    pub fn active_query(&self) -> &str {
        match self.tab {
            Tab::Overview => &self.query,
            Tab::Deps => &self.tree.query,
            Tab::RevDeps => &self.rev.query,
            Tab::Graph => &self.graph_query,
        }
    }

    pub fn edit_active_query(&mut self, edit: QueryEdit) {
        let query = match self.tab {
            Tab::Overview => &mut self.query,
            Tab::Deps => &mut self.tree.query,
            Tab::RevDeps => &mut self.rev.query,
            Tab::Graph => &mut self.graph_query,
        };
        match edit {
            QueryEdit::Push(c) => query.push(c),
            QueryEdit::Backspace => {
                query.pop();
            }
            QueryEdit::Clear => query.clear(),
        }
        match self.tab {
            Tab::Overview => {
                self.pending_query = Some(self.query.clone());
                self.last_edit = Instant::now();
                self.cursor = 0;
                self.scroll = 0;
            }
            Tab::Deps => {
                self.tree.cursor = 0;
                self.tree.scroll = 0;
                self.tree.visible.clear();
                self.request_relations();
            }
            Tab::RevDeps => {
                self.rev.cursor = 0;
                self.rev.scroll = 0;
                self.rev.visible.clear();
                self.request_relations();
            }
            Tab::Graph => self.refresh_graph_filter(),
        }
        self.dirty = true;
    }

    fn request_relations(&mut self) {
        self.refresh_tree_rows();
        let direction = match self.tab {
            Tab::Deps => Direction::Dependencies,
            Tab::RevDeps => Direction::Dependents,
            _ => {
                if let Some(worker) = &self.relations {
                    worker.cancel();
                }
                self.relations_pending = false;
                return;
            }
        };
        let Some(root) = self.anchor else {
            self.relations_pending = false;
            return;
        };
        if self.relations.is_none() {
            if let Some(index) = &self.index {
                self.relations = Some(RelationWorker::spawn(Arc::clone(index)));
            }
        }
        if let Some(worker) = &self.relations {
            self.relation_ticket = worker.request(root, direction, self.active_query().to_string());
            self.relations_pending = true;
        }
    }

    fn pump_relations(&mut self) {
        let Some(reply) = self.relations.as_ref().and_then(RelationWorker::take_reply) else {
            return;
        };
        let query = match reply.key.direction {
            Direction::Dependencies => &self.tree.query,
            Direction::Dependents => &self.rev.query,
        };
        if reply.ticket != self.relation_ticket
            || Some(reply.key.root) != self.anchor
            || reply.query != *query
            || self
                .index
                .as_ref()
                .is_none_or(|i| i.snapshot_id() != reply.key.snapshot)
        {
            return;
        }
        match reply.key.direction {
            Direction::Dependencies => {
                self.tree.relations = Some(reply.set);
                self.tree.visible = reply.visible;
            }
            Direction::Dependents => {
                self.rev.relations = Some(reply.set);
                self.rev.visible = reply.visible;
            }
        }
        self.relations_pending = false;
        self.refresh_tree_rows();
        self.dirty = true;
    }

    fn refresh_tree_rows(&mut self) {
        let (Some(index), Some(root)) = (&self.index, self.anchor) else {
            return;
        };
        self.tree.rows = Arc::new(crate::ui::tree::dep_rows(
            index,
            &self.tree,
            root,
            usize::MAX,
        ));
        self.rev.rows = Arc::new(crate::ui::tree::rev_rows(
            index,
            &self.rev,
            root,
            usize::MAX,
        ));
    }

    /// Search text and navigation commands are distinct on every tab.
    fn request_graph(&mut self, root: u32, depth: u8, selected: Option<u32>) {
        let Some(index) = &self.index else {
            return;
        };
        if root as usize >= index.len() {
            return;
        }
        if self.graph_requested == Some((root, depth))
            && (self.graph_pending || self.graph.laid_out)
        {
            return;
        }
        if self.graph_worker.is_none() {
            self.graph_worker = Some(GraphWorker::spawn(Arc::clone(index)));
        }
        self.graph_ticket = self.graph_worker.as_ref().unwrap().request(root, depth);
        self.graph_requested = Some((root, depth));
        self.graph_pending = true;
        self.graph_restore_selection = selected;
        self.graph = GraphView::new(root, depth);
        self.graph_visible.clear();
        self.graph_scroll = 0;
        self.graph_details_scroll = 0;
        self.dirty = true;
    }

    fn pump_graph(&mut self) {
        let Some(reply) = self.graph_worker.as_ref().and_then(GraphWorker::take_reply) else {
            return;
        };
        if reply.ticket != self.graph_ticket
            || self.graph_requested != Some((reply.view.root, reply.view.depth))
            || self
                .index
                .as_ref()
                .is_none_or(|index| index.snapshot_id() != reply.snapshot)
        {
            return;
        }
        self.graph = reply.view;
        if let Some(id) = self.graph_restore_selection.take() {
            self.graph.selected = self.graph.nodes.iter().position(|n| *n == id).unwrap_or(0);
        }
        self.graph_pending = false;
        self.refresh_graph_filter();
        self.dirty = true;
    }

    fn refresh_graph_filter(&mut self) {
        let Some(index) = &self.index else {
            return;
        };
        let selected = self.graph.selected_id();
        self.graph_visible = self
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, &id)| {
                let p = &index.packages[id as usize];
                crate::relations::matches_relation(&p.name, &p.version, &self.graph_query)
                    .then_some(i)
            })
            .collect();
        self.graph_visible.sort_unstable_by(|&a, &b| {
            let pa = &index.packages[self.graph.nodes[a] as usize];
            let pb = &index.packages[self.graph.nodes[b] as usize];
            pa.name
                .cmp(&pb.name)
                .then_with(|| pa.version.cmp(&pb.version))
                .then(pa.id.cmp(&pb.id))
        });
        self.graph.selected = self
            .graph_visible
            .iter()
            .copied()
            .find(|&i| Some(self.graph.nodes[i]) == selected)
            .or_else(|| self.graph_visible.first().copied())
            .unwrap_or(usize::MAX);
        self.graph_scroll = 0;
        self.graph_details_scroll = 0;
    }

    fn graph_select_delta(&mut self, delta: i32) {
        if self.graph_visible.is_empty() {
            return;
        }
        let pos = self
            .graph_visible
            .iter()
            .position(|i| *i == self.graph.selected)
            .unwrap_or(0);
        let next = (pos as i32 + delta).rem_euclid(self.graph_visible.len() as i32) as usize;
        self.graph.selected = self.graph_visible[next];
        self.graph_details_scroll = 0;
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if self.help_open {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::F(1)) {
                self.help_open = false;
                self.dirty = true;
            }
            return;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.switch_tab(
                    if key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.tab.prev()
                    } else {
                        self.tab.next()
                    },
                );
                return;
            }
            KeyCode::F(1) => {
                self.help_open = true;
                self.dirty = true;
                return;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.edit_active_query(QueryEdit::Clear);
                return;
            }
            _ => {}
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return;
        }
        if self.mode == InputMode::Search {
            match key.code {
                KeyCode::Char(c) if !c.is_control() => self.edit_active_query(QueryEdit::Push(c)),
                KeyCode::Backspace => self.edit_active_query(QueryEdit::Backspace),
                KeyCode::Enter | KeyCode::Esc => {
                    self.mode = InputMode::Navigate;
                    self.dirty = true;
                }
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End => self.navigate_key(key),
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Char('/') {
            self.mode = InputMode::Search;
            self.dirty = true;
            return;
        }
        if key.code == KeyCode::Backspace {
            self.edit_active_query(QueryEdit::Backspace);
            return;
        }
        self.navigate_key(key);
    }

    fn navigate_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
            }
            KeyCode::Esc => {
                if self.help_open {
                    self.help_open = false;
                } else if !self.active_query().is_empty() {
                    self.edit_active_query(QueryEdit::Clear);
                } else if self.tab == Tab::Graph && !self.graph_history.is_empty() {
                    if let Some(context) = self.graph_history.pop() {
                        self.graph_query = context.query;
                        self.request_graph(context.root, context.depth, context.selected_id);
                    }
                } else if self.tab != Tab::Overview {
                    self.switch_tab(Tab::Overview);
                }
                self.dirty = true;
            }
            KeyCode::Char('?') => {
                self.help_open = !self.help_open;
                self.dirty = true;
            }
            KeyCode::Char('e') if self.tab == Tab::Graph => {
                self.edge_mode = self.edge_mode.next();
                self.dirty = true;
            }
            KeyCode::Char('[') if self.tab == Tab::Graph => {
                self.graph_details_scroll = self.graph_details_scroll.saturating_sub(1);
                self.dirty = true;
            }
            KeyCode::Char(']') if self.tab == Tab::Graph => {
                self.graph_details_scroll = self.graph_details_scroll.saturating_add(1);
                self.dirty = true;
            }
            KeyCode::Char('l') if self.tab == Tab::Graph => {
                self.graph_labels = !self.graph_labels;
                self.dirty = true;
            }
            KeyCode::Char('T') => {
                // On terminals uppercase T always carries SHIFT; keep a
                // single forward cycle so the key never types into search.
                self.theme_idx = (self.theme_idx + 1) % crate::theme::THEMES.len();
                let _ = crate::theme::save_theme(self.theme_idx);
                self.dirty = true;
            }
            KeyCode::Char('R') => {
                self.rebuild();
            }
            KeyCode::Tab => {
                self.switch_tab(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.tab.prev()
                } else {
                    self.tab.next()
                });
            }
            KeyCode::Char('q') => {
                self.quit = true;
            }
            KeyCode::Char('1') => self.switch_tab(Tab::Overview),
            KeyCode::Char('2') => self.switch_tab(Tab::Deps),
            KeyCode::Char('3') => self.switch_tab(Tab::RevDeps),
            KeyCode::Char('4') => self.switch_tab(Tab::Graph),
            KeyCode::Char('d') => self.switch_tab(Tab::Deps),
            KeyCode::Char('r') => self.switch_tab(Tab::RevDeps),
            KeyCode::Char('v') => self.switch_tab(Tab::Graph),
            KeyCode::Char('o') => {
                self.open_homepage();
            }
            KeyCode::Char('g') => {
                if self.tab == Tab::Graph {
                    let Some(id) = self.selected_id() else { return };
                    self.graph_history.clear();
                    self.graph_query.clear();
                    self.request_graph(id, self.graph.depth, None);
                    self.refresh_graph_filter();
                    self.dirty = true;
                } else {
                    self.jump_top();
                }
            }
            KeyCode::Char('G') => {
                if self.tab != Tab::Graph {
                    self.jump_bottom();
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if self.tab == Tab::Graph {
                    self.graph_depth_delta(1);
                }
            }
            KeyCode::Char('-') => {
                if self.tab == Tab::Graph {
                    self.graph_depth_delta(-1);
                }
            }
            KeyCode::Char('j') => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(1);
                    self.dirty = true;
                } else {
                    self.move_cursor(1);
                }
            }
            KeyCode::Char('k') => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(-1);
                    self.dirty = true;
                } else {
                    self.move_cursor(-1);
                }
            }
            KeyCode::Char('h') => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(-1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        let set = match self.tab {
                            Tab::Deps => &mut self.tree.expanded,
                            _ => &mut self.rev.expanded,
                        };
                        set.remove(&key);
                        self.refresh_tree_rows();
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Char('l') => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        self.toggle_row(key);
                    }
                }
            }
            KeyCode::Up => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(-1);
                    self.dirty = true;
                } else {
                    self.move_cursor(-1);
                }
            }
            KeyCode::Down => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(1);
                    self.dirty = true;
                } else {
                    self.move_cursor(1);
                }
            }
            KeyCode::Left => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(-1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        let set = match self.tab {
                            Tab::Deps => &mut self.tree.expanded,
                            _ => &mut self.rev.expanded,
                        };
                        set.remove(&key);
                        self.refresh_tree_rows();
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Right => {
                if self.tab == Tab::Graph {
                    self.graph_select_delta(1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        self.toggle_row(key);
                    }
                }
            }
            KeyCode::Enter => {
                if self.tab == Tab::Graph {
                    self.graph_follow();
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        self.toggle_row(key);
                    }
                }
            }
            KeyCode::PageUp => {
                self.page_cursor(10, false);
            }
            KeyCode::PageDown => {
                self.page_cursor(10, true);
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.pending_query = Some(self.query.clone());
                self.last_edit = Instant::now();
                self.cursor = 0;
                self.scroll = 0;
                self.reset_tree_positions();
                self.dirty = true;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.pending_query = Some(String::new());
                self.last_edit = Instant::now();
                self.cursor = 0;
                self.scroll = 0;
                self.reset_tree_positions();
                self.dirty = true;
            }
            KeyCode::Home => self.jump_top(),
            KeyCode::End => self.jump_bottom(),
            _ => {}
        }
    }

    /// The NodeKey of the tree row under the cursor (Deps/RevDeps tabs).
    fn cursor_key(&self) -> Option<NodeKey> {
        let rows = if self.tab == Tab::Deps {
            &self.tree.rows.rows
        } else {
            &self.rev.rows.rows
        };
        rows.get(self.current_cursor())
            .filter(|r| !r.cycle)
            .map(|r| r.key.clone())
    }

    /// Prepare for shutdown: cancel loader, drop the event channel.
    pub fn shutdown(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.events = None;
        if let Some(loader) = self.loader.take() {
            let _ = loader.join();
        }
        self.search = None;
        self.relations = None;
        self.graph_worker = None;
    }

    /// Ensure the cursor is valid after results shrink (e.g. new reply).
    pub fn clamp_cursor(&mut self) {
        let overview = self.cursor.min(self.results.len().saturating_sub(1));
        if overview != self.cursor {
            self.cursor = overview;
            self.reset_tree_positions();
        }
        self.scroll = self.scroll.min(self.results.len().saturating_sub(1));
        let len = self.row_count();
        self.set_current_cursor(self.current_cursor().min(len.saturating_sub(1)));
        match self.tab {
            Tab::Deps => self.tree.scroll = self.tree.scroll.min(len.saturating_sub(1)),
            Tab::RevDeps => self.rev.scroll = self.rev.scroll.min(len.saturating_sub(1)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod homepage_tests {
    use super::is_web_homepage;

    #[test]
    fn only_web_homepages_can_be_launched() {
        assert!(is_web_homepage("https://gnu.org/software/emacs/"));
        assert!(is_web_homepage("http://example.org"));
        for url in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "--help",
            "https://",
            "https:///file",
            "https://example.org\n",
        ] {
            assert!(!is_web_homepage(url), "{url:?}");
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn dependencies_search_preserves_overview_query_and_selection() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.query = "emacs".into();
        app.switch_tab(Tab::Deps);
        app.on_key(key(KeyCode::Char('/')));
        for c in "zlib".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(app.query, "emacs");
        assert_eq!(app.selected_id(), Some(0));
    }

    #[test]
    fn initial_search_treats_shortcuts_as_text() {
        let mut app = fixture_app();
        for c in "jkqTR1234+-/?".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(app.query, "jkqTR1234+-/?");
        assert!(!app.quit);
        assert!(!app.help_open);
    }

    #[test]
    fn terminal_graph_starts_at_depth_one_with_focused_edges() {
        let mut app = browse_app();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        assert_eq!(app.graph.depth, 1);
        assert_eq!(app.edge_mode, EdgeMode::Focus);
    }

    #[test]
    fn narrow_graph_prioritizes_a_readable_projected_package_list() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let text = screen(&mut app, 80, 24).join("\n");
        assert!(text.contains("projected packages"), "{text}");
        assert!(text.contains("emacs") && text.contains("zlib"));
    }

    #[test]
    fn wide_graph_uses_fine_dots_instead_of_solid_edge_blocks() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let text = screen(&mut app, 130, 36).join("\n");
        assert!(
            text.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
            "{text}"
        );
        assert!(
            !text.contains(['▀', '▄', '█']),
            "solid blocks obscure the graph: {text}"
        );
        assert!(text.contains("projected packages") && text.contains("zlib"));
    }

    #[test]
    fn graph_detail_scroll_survives_event_loop_clamping() {
        let mut app = browse_app();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        app.on_key(key(KeyCode::Char(']')));
        app.on_tick();
        app.clamp_cursor();
        assert_eq!(app.graph_details_scroll, 1);
    }

    #[test]
    fn overview_marks_old_rows_while_search_is_pending() {
        let mut app = browse_app();
        type_text(&mut app, "zzzzz");
        let text = screen(&mut app, 120, 35).join("\n");
        assert!(
            text.contains("Searching") && text.contains("previous results"),
            "{text}"
        );
    }

    #[test]
    fn overview_exposes_unverified_index_origin() {
        let mut app = browse_app();
        let text = screen(&mut app, 120, 35).join("\n");
        assert!(text.contains("Guix origin unverified"), "{text}");
    }

    #[test]
    fn graph_list_exposes_all_relation_categories() {
        let doc = serde_json::from_str(include_str!("../tests/fixtures/identity.json")).unwrap();
        let mut app = fixture_app();
        app.attach_index(Arc::new(Index::from_doc(doc, 0).unwrap()), true, false);
        wait_for_reply(&mut app);
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let text = screen(&mut app, 80, 24).join("\n");
        assert!(text.contains("I/P/N"), "{text}");
    }

    #[test]
    fn graph_filter_does_not_layout_or_follow_and_history_restores_selection() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let ticket = app.graph_ticket;
        let pos = app.graph.pos.clone();
        type_text(&mut app, "zlib");
        assert_eq!(app.graph.selected_id(), Some(4));
        assert_eq!(app.graph.root, 0);
        assert_eq!(app.anchor, Some(0));
        assert!(app.query.is_empty());
        app.on_key(key(KeyCode::Enter)); // finish editing, do not follow
        assert_eq!(app.graph.root, 0);
        let _ = screen(&mut app, 120, 35);
        assert_eq!(app.graph_ticket, ticket);
        assert_eq!(app.graph.pos, pos);
        app.on_key(key(KeyCode::Enter));
        wait_for_graph(&mut app);
        assert_eq!(app.graph.root, 4);
        app.on_key(key(KeyCode::Esc));
        wait_for_graph(&mut app);
        assert_eq!(app.graph.root, 0);
        assert_eq!(app.graph_query, "zlib");
        assert_eq!(app.graph.selected_id(), Some(4));
        app.edit_active_query(QueryEdit::Clear);
        type_text(&mut app, "no-such-node");
        assert_eq!(app.graph.selected_id(), None);
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.graph.root, 0);
    }

    #[test]
    fn help_is_modal_on_every_tab() {
        let mut app = browse_app();
        for tab in Tab::ALL {
            app.switch_tab(tab);
            app.on_key(key(KeyCode::F(1)));
            assert!(app.help_open);
            let query = app.query.clone();
            app.on_key(key(KeyCode::Char('x')));
            app.on_key(key(KeyCode::Backspace));
            assert_eq!(app.query, query);
            app.on_key(key(KeyCode::Esc));
            assert!(!app.help_open);
        }
    }

    #[test]
    fn every_tab_keeps_text_mode_separate_and_escape_is_two_stage() {
        for tab in Tab::ALL {
            let mut app = browse_app();
            app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
            app.switch_tab(tab);
            let theme = app.theme_idx;
            let root = app.selected_id();
            app.on_key(key(KeyCode::Char('/')));
            type_text(&mut app, "jkh lqTR123+-é/?");
            assert_eq!(app.active_query(), "jkh lqTR123+-é/?");
            assert!(!app.quit && !app.help_open);
            assert_eq!(app.theme_idx, theme);
            assert!(matches!(app.phase, Phase::Ready { .. }));
            if tab != Tab::Overview {
                assert_eq!(app.selected_id(), root);
                assert!(app.query.is_empty());
            }
            app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
            app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
            assert_eq!(app.active_query(), "jkh lqTR123+-é/?");
            app.on_key(key(KeyCode::Backspace));
            assert_eq!(app.active_query(), "jkh lqTR123+-é/");
            app.on_key(key(KeyCode::Esc));
            assert_eq!(app.mode, InputMode::Navigate);
            assert_eq!(app.active_query(), "jkh lqTR123+-é/");
            app.on_key(key(KeyCode::Esc));
            assert!(app.active_query().is_empty());
            assert_eq!(app.tab, tab);
            app.on_key(key(KeyCode::BackTab));
            assert_eq!(app.tab, tab.prev());
            assert_eq!(app.mode, InputMode::Navigate);
        }
    }

    #[test]
    fn enter_finishes_search_without_expanding_and_rows_are_cached() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.switch_tab(Tab::Deps);
        wait_for_relations(&mut app);
        app.tree.cursor = 1;
        let expanded = app.tree.expanded.clone();
        app.on_key(key(KeyCode::Char('/')));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.mode, InputMode::Navigate);
        assert_eq!(app.tree.expanded, expanded);
        let rows = Arc::clone(&app.tree.rows);
        let _ = screen(&mut app, 80, 24);
        app.clamp_cursor();
        assert!(Arc::ptr_eq(&rows, &app.tree.rows));
        app.switch_tab(Tab::RevDeps);
        app.switch_tab(Tab::Deps);
        assert_eq!(app.tree.cursor, 1);
    }

    #[test]
    fn replacing_snapshot_discards_local_ids_history_and_workers() {
        let mut app = browse_app();
        app.switch_tab(Tab::Deps);
        type_text(&mut app, "em");
        let doc = serde_json::from_str(include_str!("../tests/fixtures/identity.json")).unwrap();
        app.attach_index(Arc::new(Index::from_doc(doc, 0).unwrap()), false, false);
        assert_eq!(app.tab, Tab::Overview);
        assert_eq!(app.anchor, None);
        assert!(
            app.graph_history.is_empty() && app.tree.query.is_empty() && app.rev.query.is_empty()
        );
        wait_for_reply(&mut app);
        assert_eq!(app.index.as_ref().unwrap().len(), 6);
    }

    #[test]
    fn dependency_and_reverse_filters_reach_the_entire_collapsed_closure() {
        let mut app = fixture_app();
        let packages: Vec<_> = (0..2502u32)
            .map(|id| {
                serde_json::json!({
                    "id":id, "name":format!("p{id:05}"), "version":"1",
                    "inputs":if id < 2501 { vec![id+1] } else { vec![] }
                })
            })
            .collect();
        let doc = serde_json::from_value(serde_json::json!({
            "header":{"schema":4,"package_count":2502},"packages":packages
        }))
        .unwrap();
        app.attach_index(Arc::new(Index::from_doc(doc, 0).unwrap()), false, false);
        app.query = "p00000".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        app.switch_tab(Tab::Deps);
        type_text(&mut app, "p02501 1");
        assert_eq!(app.row_count(), 1);
        assert_eq!(app.tree.rows.rows[0].id, 2501);
        assert_eq!(app.tree.rows.rows[0].transitive_depth, Some(2501));
        assert!(app.tree.expanded.is_empty());
        assert!(screen(&mut app, 80, 24).join("\n").contains("p02501"));
        app.switch_tab(Tab::Overview);
        app.query = "p02501".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "p00000");
        assert_eq!(app.row_count(), 1);
        assert_eq!(app.rev.rows.rows[0].id, 0);
        assert_eq!(app.rev.rows.rows[0].transitive_depth, Some(2501));
        assert!(!app.rev.trans_open);
    }

    fn fixture_app() -> App {
        let document = serde_json::from_str(include_str!("../tests/fixtures/small.json")).unwrap();
        let index = Arc::new(Index::from_doc(document, 0).unwrap());
        App {
            search: Some(SearchWorker::spawn(Arc::clone(&index))),
            relations: Some(RelationWorker::spawn(Arc::clone(&index))),
            graph_worker: Some(GraphWorker::spawn(Arc::clone(&index))),
            graph_visible: Vec::new(),
            graph_scroll: 0,
            graph_details_scroll: 0,
            graph_ticket: 0,
            graph_requested: None,
            graph_pending: false,
            graph_restore_selection: None,
            mode: InputMode::Search,
            anchor: None,
            graph_query: String::new(),
            graph_history: Vec::new(),
            relation_ticket: 0,
            relations_pending: false,
            index: Some(index),
            phase: Phase::Ready {
                fresh: true,
                unkeyed: false,
            },
            query: String::new(),
            pending_query: None,
            last_edit: Instant::now(),
            results: Vec::new(),
            rendered_ticket: 0,
            requested_ticket: 0,
            cursor: 0,
            scroll: 0,
            tab: Tab::Overview,
            tree: TreeState::default(),
            rev: RevState::default(),
            graph: GraphView::default(),
            graph_anchor: None,
            theme_idx: 0,
            help_open: false,
            dirty: false,
            tick: 0,
            size: (80, 24),
            quit: false,
            events: None,
            cancel: Arc::new(AtomicBool::new(false)),
            loader: None,
            graph_dirty: false,
            edge_mode: EdgeMode::default(),
            graph_labels: true,
        }
    }

    fn wait_for_reply(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while app.rendered_ticket < app.requested_ticket {
            app.pump_search();
            assert!(Instant::now() < deadline, "search worker did not reply");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn edited_query_rejects_the_previous_reply() {
        let mut app = fixture_app();
        app.dispatch_query(String::new());
        app.edit_active_query(QueryEdit::Push('e'));
        wait_for_reply(&mut app);
        assert!(
            app.results.is_empty(),
            "old browse results replaced the edited query"
        );
        assert!(
            app.search_pending(),
            "debounced query must keep polling responsive"
        );
        app.pending_query = None;
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        assert!(!app.results.is_empty());
        assert!(!app.search_pending());
    }

    #[test]
    fn newest_request_controls_results_and_idle_polling() {
        let mut app = fixture_app();
        app.query = "emacs".into();
        app.dispatch_query(app.query.clone());
        app.query = "zlib".into();
        app.dispatch_query(app.query.clone());
        assert!(app.search_pending());
        wait_for_reply(&mut app);
        assert_eq!(app.selected_pkg().unwrap().name.as_ref(), "zlib");
        assert!(!app.search_pending());
    }

    fn browse_app() -> App {
        let mut app = fixture_app();
        app.dispatch_query(String::new());
        wait_for_reply(&mut app);
        app.mode = InputMode::Navigate;
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(app: &mut App, text: &str) {
        if app.mode != InputMode::Search {
            app.on_key(key(KeyCode::Char('/')));
        }
        for c in text.chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        wait_for_relations(app);
    }

    fn wait_for_relations(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while app.relations_pending {
            app.pump_relations();
            assert!(Instant::now() < deadline, "relation worker did not reply");
            std::thread::yield_now();
        }
    }

    #[test]
    fn reverse_filter_keeps_overview_search_selection_and_results() {
        let mut app = browse_app();
        let zlib = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.cursor = zlib;
        app.scroll = 2;
        let original_results: Vec<u32> = app.results.iter().map(|h| h.hit.id).collect();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "EMACS");

        assert_eq!(app.query, "");
        assert_eq!(
            app.results.iter().map(|h| h.hit.id).collect::<Vec<_>>(),
            original_results
        );
        assert_eq!(app.cursor, zlib);
        assert_eq!(app.scroll, 2);
        assert_eq!(app.selected_id(), Some(4));
        assert_eq!(app.row_count(), 3);
        let lines = screen(&mut app, 80, 24);
        assert!(lines
            .iter()
            .any(|line| line.contains("Filter dependents: EMACS")));
        assert!(!lines.iter().any(|line| line.contains("gtk+")));
    }

    #[test]
    fn reverse_filter_zero_matches_navigation_clear_and_tab_roundtrip() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "zz");
        assert_eq!(app.row_count(), 0);
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::PageDown));
        assert_eq!(app.rev.cursor, 0);
        assert!(screen(&mut app, 80, 24)
            .iter()
            .any(|line| line.contains("No matching dependents")));
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.row_count(), 0);
        app.on_key(key(KeyCode::Esc)); // finish editing without clearing
        app.on_key(key(KeyCode::Esc)); // clear local filter
        assert_eq!(app.tab, Tab::RevDeps);
        assert!(app.row_count() > 0);
        type_text(&mut app, "em");
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.tab, Tab::RevDeps);
        assert_eq!(app.selected_id(), Some(4));
        assert_eq!(app.row_count(), 3);
    }

    #[test]
    fn reverse_filter_finds_collapsed_transitive_and_unicode_names() {
        let mut doc: crate::model::IndexDoc =
            serde_json::from_str(include_str!("../tests/fixtures/small.json")).unwrap();
        doc.packages[0].inputs.retain(|id| *id != 4);
        doc.packages[2].name = "Éditeur".into();
        let index = Arc::new(Index::from_doc(doc, 0).unwrap());
        let mut app = fixture_app();
        app.search = Some(SearchWorker::spawn(Arc::clone(&index)));
        app.relations = Some(RelationWorker::spawn(Arc::clone(&index)));
        app.index = Some(index);
        app.dispatch_query(String::new());
        wait_for_reply(&mut app);
        app.cursor = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.switch_tab(Tab::RevDeps);
        assert!(!app.rev.trans_open);

        type_text(&mut app, "EMACS");
        let lines = screen(&mut app, 80, 24).join("\n");
        assert_eq!(app.row_count(), 2);
        assert!(lines.contains("emacs-minimal"));
        assert!(lines.contains("emacs 30.2 d2"), "{lines}");
        assert!(!app.rev.trans_open);
        let capped = crate::ui::tree::rev_rows(
            app.index.as_ref().unwrap(),
            &app.rev,
            app.rev_root_id().unwrap(),
            4,
        );
        assert_eq!(
            capped.rows.len(),
            2,
            "filter must run before the visible row cap"
        );

        app.on_key(key(KeyCode::Esc));
        app.on_key(key(KeyCode::Esc));
        assert!(!app.rev.trans_open);
        assert_eq!(app.row_count(), 5);
        type_text(&mut app, "édi");
        assert_eq!(app.row_count(), 1);
        assert!(screen(&mut app, 80, 24).join("\n").contains("Éditeur"));
    }

    #[test]
    fn reverse_filter_pins_root_through_pending_global_reply() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.query = "gtk+".into();
        app.dispatch_query(app.query.clone());
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "em");
        wait_for_reply(&mut app);

        assert_eq!(app.query, "gtk+");
        assert_eq!(app.selected_id(), Some(4));
        assert_eq!(app.rev_root_id(), Some(4));
        assert_eq!(app.row_count(), 3);
        let lines = screen(&mut app, 80, 24);
        assert!(lines
            .iter()
            .any(|line| line.contains("who depends on zlib")));
        assert!(!lines.last().unwrap().contains("1 matches"));
        assert!(
            lines.last().unwrap().contains("Ctrl+C quit"),
            "{:?}",
            lines.last().unwrap()
        );

        app.switch_tab(Tab::Overview);
        wait_for_reply(&mut app);
        app.switch_tab(Tab::RevDeps);
        assert_eq!(app.rev_root_id(), Some(3));
        assert!(app.rev.query.is_empty());
    }

    #[test]
    fn reverse_filter_accepts_initial_command_letters() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "jkh lq");
        assert_eq!(app.rev.query, "jkh lq");
        assert_eq!(app.tab, Tab::RevDeps);
        assert!(!app.quit);
        assert_eq!(app.query, "");
    }

    #[test]
    fn reverse_filter_ctrl_u_backtab_and_overview_reselection() {
        let mut app = browse_app();
        app.cursor = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "em");
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(app.rev.query.is_empty());
        assert_eq!(app.query, "");

        type_text(&mut app, "em");
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
        assert_eq!(app.tab, Tab::Deps);
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.tab, Tab::RevDeps);
        assert_eq!(app.rev.query, "em");

        app.switch_tab(Tab::Overview);
        app.cursor = app.results.iter().position(|h| h.hit.id == 3).unwrap();
        app.switch_tab(Tab::RevDeps);
        assert_eq!(app.rev_root_id(), Some(3));
        assert!(app.rev.query.is_empty());
    }

    #[test]
    fn reverse_filter_ignores_modified_characters_without_editing_overview_query() {
        let mut app = browse_app();
        app.query = "gtk+".into();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "em");

        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT));
        app.on_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert_eq!(app.query, "gtk+");
        assert_eq!(app.rev.query, "em");
    }

    #[test]
    fn reverse_help_blocks_query_editing_and_esc_closes_help() {
        let mut app = browse_app();
        app.query = "gtk+".into();
        app.switch_tab(Tab::RevDeps);
        type_text(&mut app, "em");
        app.on_key(key(KeyCode::F(1)));
        assert!(app.help_open);

        app.on_key(key(KeyCode::Backspace));
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.query, "gtk+");
        assert_eq!(app.rev.query, "em");
        app.on_key(key(KeyCode::Esc));
        assert!(!app.help_open);
        assert_eq!(app.tab, Tab::RevDeps);
        assert_eq!(app.rev.query, "em");
        app.on_key(key(KeyCode::F(1)));
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.quit);
        assert_eq!(app.query, "gtk+");
    }

    #[test]
    fn tree_navigation_preserves_nonfirst_overview_package_across_tabs() {
        let mut app = browse_app();
        let zlib = app.results.iter().position(|h| h.hit.id == 4).unwrap();
        app.cursor = zlib;
        app.scroll = 2;
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.selected_id(), Some(4));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::PageDown));
        app.clamp_cursor();
        assert_eq!(app.selected_id(), Some(4));
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::PageUp));
        app.clamp_cursor();
        assert_eq!(app.selected_id(), Some(4));
        app.on_key(key(KeyCode::Tab));
        app.clamp_cursor();
        assert_eq!(app.selected_id(), Some(4));
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.cursor, zlib);
        assert_eq!(app.scroll, 2);
        assert_eq!(app.selected_id(), Some(4));
    }

    #[test]
    fn tree_arrows_and_pages_move_the_tree_row_only() {
        let mut app = browse_app();
        let gtk = app.results.iter().position(|h| h.hit.id == 3).unwrap();
        assert!(gtk > 0);
        app.cursor = gtk;
        app.switch_tab(Tab::Deps);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.tree.cursor, 1);
        app.on_key(key(KeyCode::PageDown));
        assert_eq!(app.tree.cursor, app.row_count() - 1);
        app.on_key(key(KeyCode::PageUp));
        assert_eq!(app.tree.cursor, 0);
        assert_eq!(app.selected_id(), Some(3));
        app.switch_tab(Tab::RevDeps);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.rev.cursor, 1);
        assert_eq!(app.selected_id(), Some(3));
    }

    #[test]
    fn empty_and_shortened_results_clear_or_clamp_selection_and_graph() {
        let mut app = browse_app();
        app.cursor = app.results.len() - 1;
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let anchor = app.selected_id();
        app.results.truncate(2);
        app.clamp_cursor();
        assert_eq!(app.cursor, 1);
        assert_eq!(
            app.selected_id(),
            anchor,
            "inactive Overview cannot change a pinned root"
        );
        app.switch_tab(Tab::Overview);
        assert_eq!(app.selected_id(), Some(app.results[1].hit.id));
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        app.ensure_graph();
        assert_eq!(app.graph.root, app.results[1].hit.id);
        app.results.clear();
        app.switch_tab(Tab::Overview);
        app.clamp_cursor();
        app.ensure_graph();
        assert_eq!(app.selected_id(), None);
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        assert!(app.graph.nodes.is_empty());
    }

    #[test]
    fn graph_follow_and_depth_survive_draw_and_tab_round_trip_until_refocus() {
        let mut app = browse_app();
        let emacs = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.cursor = emacs;
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let next = app.graph.nodes.iter().position(|id| *id != 0).unwrap();
        let followed = app.graph.nodes[next];
        app.graph.selected = next;
        app.graph_follow();
        app.graph_depth_delta(1);
        let depth = app.graph.depth;
        let _ = screen(&mut app, 80, 24);
        app.switch_tab(Tab::Deps);
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        let _ = screen(&mut app, 80, 24);
        assert_eq!(app.graph.root, followed);
        assert_eq!(app.graph.depth, depth);
        app.on_key(key(KeyCode::Char('g')));
        assert_eq!(app.graph.root, 0);
        assert_eq!(app.graph.depth, depth);
    }

    #[test]
    fn accepted_search_reply_reanchors_graph_to_new_result() {
        let mut app = browse_app();
        app.query = "zlib".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        assert_eq!(app.selected_id(), Some(4));
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        app.ensure_graph();
        assert_eq!(app.graph.root, 4);
        app.switch_tab(Tab::Overview);
        app.query = "unfindable-package-name".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        assert_eq!(app.selected_id(), None);
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        assert!(app.graph.nodes.is_empty());
    }

    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
        wait_for_graph(app);
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn wait_for_graph(app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while app.graph_pending {
            app.pump_graph();
            assert!(Instant::now() < deadline, "graph worker did not reply");
            std::thread::yield_now();
        }
    }

    #[test]
    fn search_query_is_visible_in_the_bordered_header() {
        let mut app = browse_app();
        app.query = "visible-query".into();
        let lines = screen(&mut app, 80, 24);
        assert!(lines
            .iter()
            .any(|line| line.contains("Search: visible-query")));
    }

    #[test]
    fn graph_canvas_labels_root_and_selected_node_at_eighty_columns() {
        let mut app = browse_app();
        let emacs = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.cursor = emacs;
        app.switch_tab(Tab::Graph);
        wait_for_graph(&mut app);
        app.graph.selected = app.graph.nodes.iter().position(|id| *id == 4).unwrap();
        let lines = screen(&mut app, 80, 24);
        let canvas = lines[6..21].join("\n");
        assert!(
            canvas.contains("emacs"),
            "root missing from canvas: {canvas}"
        );
        assert!(
            canvas.contains("zlib"),
            "selection missing from canvas: {canvas}"
        );
    }

    #[test]
    fn tiny_terminal_draw_is_safe_on_every_tab() {
        let mut app = browse_app();
        for tab in Tab::ALL {
            app.switch_tab(tab);
            let _ = screen(&mut app, 12, 4);
        }
    }
}
