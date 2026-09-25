//! Dependency and reverse-dependency trees (expandable rows).

use std::collections::HashSet;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::{App, NodeKey, NodeKind, RevState, TreeState};
use crate::index::{DepKind, Index};
use crate::theme::Theme;

/// Sentinel id for the "Transitive" section header row.
const TRANS_SECTION: u32 = u32::MAX;

/// Row cap for any flattened tree (kept honest by a truncation row).
const ROW_CAP: usize = 2000;
/// Reverse-BFS depth cap.
const REV_DEPTH: u8 = 8;
/// Nodes materialized for the transitive section.
const TRANS_CAP: usize = 500;

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub key: NodeKey,
    pub id: u32,
    pub depth: u8,
    pub kind: NodeKind,
    pub children_count: usize,
    pub is_section: bool,
    pub transitive_depth: Option<u8>,
}

/// Flatten the dependency tree of `root` (expanded nodes recurse).
pub fn dep_rows(index: &Index, state: &TreeState, root: u32, cap: usize) -> (Vec<TreeRow>, usize) {
    let cap = cap.min(ROW_CAP);
    let mut rows: Vec<TreeRow> = Vec::new();
    let Some(pkg) = index.packages.get(root as usize) else {
        return (rows, 0);
    };
    rows.push(TreeRow {
        key: (root, NodeKind::Input),
        id: root,
        depth: 0,
        kind: NodeKind::Input,
        children_count: pkg.dep_count(),
        is_section: false,
        transitive_depth: None,
    });
    let mut stack: Vec<(u32, u8, NodeKey)> = vec![(root, 0, (root, NodeKind::Input))];
    let mut path: HashSet<u32> = HashSet::new();
    path.insert(root);

    'outer: while let Some((id, depth, key)) = stack.pop() {
        let expanded = id == root || state.expanded.contains(&key);
        if expanded {
            let p = &index.packages[id as usize];
            let mut children: Vec<(u32, DepKind)> = p.deps().collect();
            children.sort_by(|a, b| {
                let na = &index.packages[a.0 as usize].name;
                let nb = &index.packages[b.0 as usize].name;
                na.cmp(nb)
            });
            for (child, kind) in children.into_iter().rev() {
                if path.contains(&child) {
                    continue; // cycle guard
                }
                let nk = match kind {
                    DepKind::Input => NodeKind::Input,
                    DepKind::Propagated => NodeKind::Propagated,
                    DepKind::Native => NodeKind::Native,
                };
                let ckey = (child, nk);
                let children_count = index.packages[child as usize].dep_count();
                rows.push(TreeRow {
                    key: ckey,
                    id: child,
                    depth: depth + 1,
                    kind: nk,
                    children_count,
                    is_section: false,
                    transitive_depth: None,
                });
                if rows.len() >= cap {
                    break 'outer;
                }
                path.insert(child);
                stack.push((child, depth + 1, ckey));
            }
        }
        path.remove(&id);
    }
    let n = rows.len();
    (rows, n)
}

/// Flatten the reverse-dependency view of `root`.
pub fn rev_rows(index: &Index, state: &RevState, root: u32, cap: usize) -> (Vec<TreeRow>, usize) {
    let cap = cap.min(ROW_CAP);
    let mut rows: Vec<TreeRow> = Vec::new();
    if index.packages.get(root as usize).is_none() {
        return (rows, 0);
    }
    rows.push(TreeRow {
        key: (root, NodeKind::RevDirect),
        id: root,
        depth: 0,
        kind: NodeKind::RevDirect,
        children_count: index.dependents_count(root),
        is_section: false,
        transitive_depth: None,
    });

    // Direct dependents (expandable -> their own dependents).
    let direct = index.dependents[root as usize].clone();
    for d in direct.iter() {
        let children_count = index.dependents_count(*d);
        rows.push(TreeRow {
            key: (*d, NodeKind::RevDirect),
            id: *d,
            depth: 1,
            kind: NodeKind::RevDirect,
            children_count,
            is_section: false,
            transitive_depth: None,
        });
    }

    // Transitive section header.
    let (trans, total_seen) = index.dependents_bfs(root, REV_DEPTH, TRANS_CAP);
    let trans_only: Vec<(u32, u8)> = trans
        .iter()
        .filter(|n| n.depth >= 2)
        .map(|n| (n.id, n.depth))
        .collect();
    let transitive_count = total_seen.saturating_sub(direct.len());
    rows.push(TreeRow {
        key: (TRANS_SECTION, NodeKind::RevTrans),
        id: TRANS_SECTION,
        depth: 0,
        kind: NodeKind::RevTrans,
        children_count: transitive_count,
        is_section: true,
        transitive_depth: None,
    });

    if state.trans_open {
        let mut trans_rows: Vec<TreeRow> = Vec::new();
        for (id, depth) in &trans_only {
            trans_rows.push(TreeRow {
                key: (*id, NodeKind::RevTrans),
                id: *id,
                depth: 1,
                kind: NodeKind::RevTrans,
                children_count: index.dependents_count(*id),
                is_section: false,
                transitive_depth: Some(*depth),
            });
        }
        trans_rows.sort_by(|a, b| {
            index.packages[a.id as usize]
                .name
                .cmp(&index.packages[b.id as usize].name)
        });
        rows.extend(trans_rows);
        if rows.len() >= cap {
            rows.truncate(cap);
        }
    }
    let n = rows.len();
    (rows, n)
}

fn draw_tree(f: &mut Frame, app: &mut App, th: &Theme, area: Rect, title: &str, reverse: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(th.border)
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(index) = app.index.as_ref() else {
        f.render_widget(
            ratatui::widgets::Paragraph::new("index loading…").style(Style::default().fg(th.muted)),
            inner,
        );
        return;
    };
    let Some(id) = app.selected_id() else {
        f.render_widget(
            ratatui::widgets::Paragraph::new("no package selected")
                .style(Style::default().fg(th.muted)),
            inner,
        );
        return;
    };

    let (rows, _) = if reverse {
        rev_rows(index, &app.rev, id, usize::MAX)
    } else {
        dep_rows(index, &app.tree, id, usize::MAX)
    };
    if inner.height < 2 {
        return;
    }
    let viewport = inner.height as usize - 1;
    let max_scroll = rows.len().saturating_sub(viewport);
    let (cursor, scroll) = if reverse {
        (app.rev.cursor, &mut app.rev.scroll)
    } else {
        (app.tree.cursor, &mut app.tree.scroll)
    };
    *scroll = (*scroll).min(max_scroll);
    if cursor < *scroll {
        *scroll = cursor;
    }
    if cursor >= *scroll + viewport {
        *scroll = cursor + 1 - viewport;
    }
    let scroll = *scroll;

    let items: Vec<ListItem> = rows
        .iter()
        .skip(scroll)
        .take(viewport)
        .map(|r| ListItem::new(render_row(index, app, th, r, reverse)))
        .collect();
    let selected = cursor.saturating_sub(scroll);
    let list = List::new(items)
        .highlight_style(th.selected)
        .highlight_symbol("▶ ");
    f.render_stateful_widget(
        list,
        inner,
        &mut ListState::default().with_selected(Some(selected.min(viewport.saturating_sub(1)))),
    );
}

pub fn draw_deps(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let title = match app.selected_pkg() {
        Some(p) => format!(
            " dependencies of {} {} — Enter expand/collapse ",
            p.name, p.version
        ),
        None => " dependencies ".to_string(),
    };
    draw_tree(f, app, th, area, &title, false);
}

pub fn draw_revs(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let title = match app.selected_pkg() {
        Some(p) => format!(
            " who depends on {} {} — Enter expand/collapse ",
            p.name, p.version
        ),
        None => " reverse dependencies ".to_string(),
    };
    draw_tree(f, app, th, area, &title, true);
}

fn render_row<'a>(
    index: &'a Index,
    app: &App,
    th: &Theme,
    row: &TreeRow,
    reverse: bool,
) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
    let indent = (row.depth as usize).saturating_sub(if row.is_section { 1 } else { 0 });
    spans.push(Span::raw("  ".repeat(indent)));

    if row.is_section {
        let open = if reverse { app.rev.trans_open } else { false };
        let glyph = if open { "▾" } else { "▸" };
        spans.push(Span::styled(
            format!(
                "{glyph} Transitive (depth 2+, {} reached)",
                row.children_count
            ),
            Style::default().fg(th.accent2).add_modifier(Modifier::BOLD),
        ));
        return Line::from(spans);
    }

    let p = &index.packages[row.id as usize];
    let expanded = match row.kind {
        NodeKind::RevDirect | NodeKind::RevTrans => app.rev.expanded.contains(&row.key),
        _ => {
            row.id == app.selected_id().unwrap_or(u32::MAX) || app.tree.expanded.contains(&row.key)
        }
    };
    let glyph = if row.children_count > 0 {
        if expanded {
            "▾"
        } else {
            "▸"
        }
    } else {
        " "
    };
    spans.push(Span::raw(format!("{glyph} ")));
    spans.push(Span::styled(
        p.name.as_ref(),
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
    ));
    if !p.version.is_empty() {
        spans.push(Span::styled(
            format!(" {}", p.version),
            Style::default().fg(th.muted),
        ));
    }

    // Badges.
    match row.kind {
        NodeKind::Propagated => spans.push(badge("P", th.badge_p)),
        NodeKind::Native => spans.push(badge("N", th.badge_n)),
        NodeKind::RevTrans => {
            if let Some(d) = row.transitive_depth {
                spans.push(Span::styled(
                    format!(" d{d}"),
                    Style::default().fg(th.accent2),
                ));
            }
        }
        _ => {}
    }

    let dependents = index.dependents_count(row.id);
    if dependents > 0 {
        spans.push(Span::styled(
            format!(" ⤴{dependents}"),
            Style::default().fg(th.badge_p),
        ));
    }

    // Synopsis preview for direct rows only, keep rows compact.
    if row.depth <= 1 && !p.synopsis.is_empty() {
        let syn: String = p.synopsis.chars().take(50).collect();
        spans.push(Span::styled(
            format!("  {syn}"),
            Style::default().fg(th.muted),
        ));
    }
    Line::from(spans)
}

fn badge(text: &str, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(
        format!(" {text}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}
