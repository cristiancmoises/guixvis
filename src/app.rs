//! Application state, phase management, and keyboard dispatch.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::graph::{GraphView, DEFAULT_DEPTH};
use crate::index::Index;
use crate::indexer::{self, IndexEvent};
use crate::search::{HighlightedHit, SearchWorker, DEBOUNCE};

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

pub type NodeKey = (u32, NodeKind);

#[derive(Debug, Default)]
pub struct TreeState {
    /// Keys of nodes whose children are expanded.
    pub expanded: HashSet<NodeKey>,
}

#[derive(Debug, Default)]
pub struct RevState {
    pub expanded: HashSet<NodeKey>,
    /// Whether the "Transitive" section is expanded.
    pub trans_open: bool,
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
    pub cursor: usize,
    pub scroll: usize,
    pub tab: Tab,
    pub tree: TreeState,
    pub rev: RevState,
    pub graph: GraphView,
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
            cursor: 0,
            scroll: 0,
            tab: Tab::Overview,
            tree: TreeState::default(),
            rev: RevState::default(),
            graph: GraphView::default(),
            theme_idx: 0,
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
        }
    }

    pub fn selected_id(&self) -> Option<u32> {
        self.results.get(self.cursor).map(|h| h.hit.id)
    }

    pub fn selected_pkg(&self) -> Option<&crate::index::Package> {
        let id = self.selected_id()?;
        self.index.as_ref()?.packages.get(id as usize)
    }

    pub fn animating(&self) -> bool {
        matches!(self.phase, Phase::Loading { .. })
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
        self.index = Some(index);
        self.phase = Phase::Ready { fresh, unkeyed };
        self.results.clear();
        self.rendered_ticket = 0;
        self.cursor = 0;
        self.scroll = 0;
        self.tree.expanded.clear();
        self.rev.expanded.clear();
        self.rev.trans_open = false;
        self.graph = GraphView::default();
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
        self.results = reply.hits;
        self.cursor = 0;
        self.scroll = 0;
        true
    }

    /// Per-tick work: flush debounced queries.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
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
            search.send(query);
        }
    }

    /// Push a keystroke into the query box.
    fn push_query(&mut self, c: char) {
        self.query.push(c);
        self.pending_query = Some(self.query.clone());
        self.last_edit = Instant::now();
        self.cursor = 0;
        self.scroll = 0;
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
        if pkg.homepage.is_empty() {
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
        self.tab = tab;
        self.scroll = 0;
        if tab == Tab::Graph {
            self.ensure_graph();
        }
        self.dirty = true;
    }

    /// (Re)build the graph view from the current selection.
    pub fn ensure_graph(&mut self) {
        let Some(index) = self.index.clone() else {
            return;
        };
        let Some(id) = self.selected_id() else {
            return;
        };
        if self.graph.root != id || !self.graph.laid_out {
            self.graph.rebuild(&index, id, DEFAULT_DEPTH);
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
                if key.0 == u32::MAX && key.1 == NodeKind::RevTrans {
                    self.rev.trans_open = !self.rev.trans_open;
                } else if self.rev.expanded.contains(&key) {
                    self.rev.expanded.remove(&key);
                } else {
                    self.rev.expanded.insert(key);
                }
            }
            _ => return,
        }
        self.dirty = true;
    }

    /// Follow the selected graph node as the new root.
    pub fn graph_follow(&mut self) {
        let Some(index) = self.index.clone() else {
            return;
        };
        let Some(id) = self.graph.selected_id() else {
            return;
        };
        self.graph.rebuild(&index, id, self.graph.depth);
        self.dirty = true;
    }

    pub fn graph_depth_delta(&mut self, delta: i8) {
        let Some(index) = self.index.clone() else {
            return;
        };
        let depth = crate::graph::clamp_depth(self.graph.depth, delta);
        if depth != self.graph.depth {
            self.graph.rebuild(&index, self.graph.root, depth);
            self.dirty = true;
        }
    }

    /// Move the list cursor, clamping scroll to keep it visible.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.row_count() as i32;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
        self.dirty = true;
    }

    pub fn page_cursor(&mut self, page: usize, down: bool) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        self.cursor = if down {
            (self.cursor + page).min(len - 1)
        } else {
            self.cursor.saturating_sub(page)
        };
        self.dirty = true;
    }

    pub fn jump_top(&mut self) {
        self.cursor = 0;
        self.dirty = true;
    }

    pub fn jump_bottom(&mut self) {
        let len = self.row_count();
        if len > 0 {
            self.cursor = len - 1;
            self.dirty = true;
        }
    }

    /// Number of navigable rows in the current tab.
    pub fn row_count(&self) -> usize {
        match self.tab {
            Tab::Overview => self.results.len(),
            Tab::Deps | Tab::RevDeps => {
                let Some(index) = self.index.as_ref() else {
                    return 0;
                };
                let Some(id) = self.selected_id() else {
                    return 0;
                };
                if self.tab == Tab::Deps {
                    crate::ui::tree::dep_rows(index, &self.tree, id, usize::MAX)
                        .0
                        .len()
                } else {
                    crate::ui::tree::rev_rows(index, &self.rev, id, usize::MAX)
                        .0
                        .len()
                }
            }
            Tab::Graph => 0,
        }
    }

    /// Handle one key event. Command character keys (d, r, v, 1-4, g, G, j,
    /// k, h, l, +, -, q) act only while the search box is empty, so typing
    /// stays free-form; arrows, Enter, Tab, PgUp/PgDn and Esc always work.
    pub fn on_key(&mut self, key: KeyEvent) {
        let idle = self.query.is_empty();
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
            }
            KeyCode::Esc => {
                if self.help_open {
                    self.help_open = false;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.pending_query = Some(String::new());
                    self.last_edit = Instant::now();
                } else if self.tab != Tab::Overview {
                    self.switch_tab(Tab::Overview);
                }
                self.dirty = true;
            }
            KeyCode::Char('?') => {
                self.help_open = !self.help_open;
                self.dirty = true;
            }
            KeyCode::Char('T') => {
                // On terminals uppercase T always carries SHIFT; keep a
                // single forward cycle so the key never types into search.
                self.theme_idx = (self.theme_idx + 1) % crate::theme::THEMES.len();
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
            KeyCode::Char('q') if idle => {
                self.quit = true;
            }
            KeyCode::Char('1') if idle => self.switch_tab(Tab::Overview),
            KeyCode::Char('2') if idle => self.switch_tab(Tab::Deps),
            KeyCode::Char('3') if idle => self.switch_tab(Tab::RevDeps),
            KeyCode::Char('4') if idle => self.switch_tab(Tab::Graph),
            KeyCode::Char('d') if idle => self.switch_tab(Tab::Deps),
            KeyCode::Char('r') if idle => self.switch_tab(Tab::RevDeps),
            KeyCode::Char('v') if idle => self.switch_tab(Tab::Graph),
            KeyCode::Char('o') if idle => {
                self.open_homepage();
            }
            KeyCode::Char('g') if idle => {
                if self.tab == Tab::Graph {
                    let Some(index) = self.index.clone() else {
                        return;
                    };
                    let Some(id) = self.selected_id() else { return };
                    self.graph.rebuild(&index, id, self.graph.depth);
                    self.dirty = true;
                } else {
                    self.jump_top();
                }
            }
            KeyCode::Char('G') if idle => {
                if self.tab != Tab::Graph {
                    self.jump_bottom();
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') if idle => {
                if self.tab == Tab::Graph {
                    self.graph_depth_delta(1);
                }
            }
            KeyCode::Char('-') if idle => {
                if self.tab == Tab::Graph {
                    self.graph_depth_delta(-1);
                }
            }
            KeyCode::Char('j') if idle => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(1);
                    self.dirty = true;
                } else {
                    self.move_cursor(1);
                }
            }
            KeyCode::Char('k') if idle => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(-1);
                    self.dirty = true;
                } else {
                    self.move_cursor(-1);
                }
            }
            KeyCode::Char('h') if idle => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(-1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        let set = match self.tab {
                            Tab::Deps => &mut self.tree.expanded,
                            _ => &mut self.rev.expanded,
                        };
                        set.remove(&key);
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Char('l') if idle => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        self.toggle_row(key);
                    }
                }
            }
            KeyCode::Up => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(-1);
                    self.dirty = true;
                } else {
                    self.move_cursor(-1);
                }
            }
            KeyCode::Down => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(1);
                    self.dirty = true;
                } else {
                    self.move_cursor(1);
                }
            }
            KeyCode::Left => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(-1);
                    self.dirty = true;
                } else if self.tab == Tab::Deps || self.tab == Tab::RevDeps {
                    if let Some(key) = self.cursor_key() {
                        let set = match self.tab {
                            Tab::Deps => &mut self.tree.expanded,
                            _ => &mut self.rev.expanded,
                        };
                        set.remove(&key);
                        self.dirty = true;
                    }
                }
            }
            KeyCode::Right => {
                if self.tab == Tab::Graph {
                    self.graph.select_delta(1);
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
                self.dirty = true;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.pending_query = Some(String::new());
                self.last_edit = Instant::now();
                self.cursor = 0;
                self.scroll = 0;
                self.dirty = true;
            }
            KeyCode::Char(c) => {
                self.push_query(c);
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// The NodeKey of the tree row under the cursor (Deps/RevDeps tabs).
    fn cursor_key(&self) -> Option<NodeKey> {
        let index = self.index.as_ref()?;
        let id = self.selected_id()?;
        let (rows, _) = if self.tab == Tab::Deps {
            crate::ui::tree::dep_rows(index, &self.tree, id, usize::MAX)
        } else {
            crate::ui::tree::rev_rows(index, &self.rev, id, usize::MAX)
        };
        rows.get(self.cursor).map(|r| r.key)
    }

    /// Prepare for shutdown: cancel loader, drop the event channel.
    pub fn shutdown(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.events = None;
        if let Some(loader) = self.loader.take() {
            let _ = loader.join();
        }
        self.search = None;
    }

    /// Ensure the cursor is valid after results shrink (e.g. new reply).
    pub fn clamp_cursor(&mut self) {
        let len = self.row_count();
        if len > 0 && self.cursor >= len {
            self.cursor = len - 1;
        }
        if len == 0 {
            self.cursor = 0;
        }
    }
}
