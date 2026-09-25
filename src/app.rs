//! Application state, phase management, and keyboard dispatch.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::graph::{EdgeMode, GraphView, DEFAULT_DEPTH};
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
}

#[derive(Debug, Default)]
pub struct RevState {
    pub expanded: HashSet<NodeKey>,
    /// Whether the "Transitive" section is expanded.
    pub trans_open: bool,
    pub cursor: usize,
    pub scroll: usize,
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
        self.results.get(self.cursor).map(|h| h.hit.id)
    }

    pub fn selected_pkg(&self) -> Option<&crate::index::Package> {
        let id = self.selected_id()?;
        self.index.as_ref()?.packages.get(id as usize)
    }

    pub fn animating(&self) -> bool {
        matches!(self.phase, Phase::Loading { .. })
    }

    /// Keep input and reply polling responsive until the current query settles.
    pub fn search_pending(&self) -> bool {
        self.pending_query.is_some() || self.requested_ticket > self.rendered_ticket
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
        self.tree.cursor = 0;
        self.tree.scroll = 0;
        self.rev.expanded.clear();
        self.rev.trans_open = false;
        self.rev.cursor = 0;
        self.rev.scroll = 0;
        self.graph = GraphView::default();
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
        if reply.ticket != self.requested_ticket || reply.query != self.query {
            return false;
        }
        let selected = self.selected_id();
        self.results = reply.hits;
        self.cursor = self.cursor.min(self.results.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.results.len().saturating_sub(1));
        if selected != self.selected_id() {
            self.reset_tree_positions();
        }
        if self.tab == Tab::Graph {
            self.ensure_graph();
        }
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
            self.requested_ticket = search.send(query);
        }
    }

    /// Push a keystroke into the query box.
    fn push_query(&mut self, c: char) {
        self.query.push(c);
        self.pending_query = Some(self.query.clone());
        self.last_edit = Instant::now();
        self.cursor = 0;
        self.scroll = 0;
        self.reset_tree_positions();
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
        self.tab = tab;
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
                self.dirty = true;
            }
            return;
        };
        let Some(index) = self.index.clone() else {
            return;
        };
        if self.graph_anchor != Some(id) || !self.graph.laid_out {
            self.graph.rebuild(&index, id, DEFAULT_DEPTH);
            self.graph_anchor = Some(id);
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

    fn current_cursor(&self) -> usize {
        match self.tab {
            Tab::Overview => self.cursor,
            Tab::Deps => self.tree.cursor,
            Tab::RevDeps => self.rev.cursor,
            Tab::Graph => self.graph.selected,
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
            Tab::Graph => {}
        }
    }

    fn reset_tree_positions(&mut self) {
        self.tree.cursor = 0;
        self.tree.scroll = 0;
        self.rev.cursor = 0;
        self.rev.scroll = 0;
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
            KeyCode::Char('e') if self.tab == Tab::Graph && idle => {
                self.edge_mode = self.edge_mode.next();
                self.dirty = true;
            }
            KeyCode::Char('l') if self.tab == Tab::Graph && idle => {
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
        rows.get(self.current_cursor()).map(|r| r.key)
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

    fn fixture_app() -> App {
        let document = serde_json::from_str(include_str!("../tests/fixtures/small.json")).unwrap();
        let index = Arc::new(Index::from_doc(document, 0).unwrap());
        App {
            search: Some(SearchWorker::spawn(Arc::clone(&index))),
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
        app.push_query('e');
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
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
        app.results.truncate(2);
        app.clamp_cursor();
        assert_eq!(app.cursor, 1);
        assert_eq!(app.selected_id(), Some(app.results[1].hit.id));
        app.ensure_graph();
        assert_eq!(app.graph.root, app.results[1].hit.id);
        app.results.clear();
        app.clamp_cursor();
        app.ensure_graph();
        assert_eq!(app.selected_id(), None);
        assert!(app.graph.nodes.is_empty());
    }

    #[test]
    fn graph_follow_and_depth_survive_draw_and_tab_round_trip_until_refocus() {
        let mut app = browse_app();
        let emacs = app.results.iter().position(|h| h.hit.id == 0).unwrap();
        app.cursor = emacs;
        app.switch_tab(Tab::Graph);
        let next = app.graph.nodes.iter().position(|id| *id != 0).unwrap();
        let followed = app.graph.nodes[next];
        app.graph.selected = next;
        app.graph_follow();
        app.graph_depth_delta(1);
        let depth = app.graph.depth;
        let _ = screen(&mut app, 80, 24);
        app.switch_tab(Tab::Deps);
        app.switch_tab(Tab::Graph);
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
        app.switch_tab(Tab::Graph);
        app.query = "zlib".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        assert_eq!(app.selected_id(), Some(4));
        app.ensure_graph();
        assert_eq!(app.graph.root, 4);
        app.query = "unfindable-package-name".into();
        app.dispatch_query(app.query.clone());
        wait_for_reply(&mut app);
        assert_eq!(app.selected_id(), None);
        assert!(app.graph.nodes.is_empty());
    }

    fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
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
