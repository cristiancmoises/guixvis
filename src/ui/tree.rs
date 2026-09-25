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

/// Only optional expanded descendants have a presentation budget. Direct
/// relations and search results remain complete and are virtualized on draw.
const ROW_CAP: usize = 2000;

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub key: NodeKey,
    pub id: u32,
    pub depth: usize,
    pub kind: NodeKind,
    pub kinds: Vec<DepKind>,
    pub children_count: usize,
    pub is_section: bool,
    pub transitive_depth: Option<usize>,
    pub cycle: bool,
}

#[derive(Debug, Default, Clone)]
pub struct TreeRows {
    pub rows: Vec<TreeRow>,
    pub materialization_limited: bool,
}

fn node_kind(kind: DepKind) -> NodeKind {
    match kind {
        DepKind::Input => NodeKind::Input,
        DepKind::Propagated => NodeKind::Propagated,
        DepKind::Native => NodeKind::Native,
    }
}

fn children(index: &Index, id: u32, reverse: bool) -> Vec<(u32, Vec<DepKind>)> {
    let mut children: Vec<_> = if reverse {
        index.dependents[id as usize]
            .iter()
            .map(|&dep| (dep, index.packages[dep as usize].dep_kinds(id)))
            .collect()
    } else {
        index.packages[id as usize]
            .deps()
            .map(|(dep, _)| (dep, index.packages[id as usize].dep_kinds(dep)))
            .collect()
    };
    children.sort_unstable_by(|a, b| {
        let pa = &index.packages[a.0 as usize];
        let pb = &index.packages[b.0 as usize];
        pa.name
            .cmp(&pb.name)
            .then_with(|| pa.version.cmp(&pb.version))
            .then(a.0.cmp(&b.0))
    });
    children
}

fn flatten(
    index: &Index,
    root: u32,
    expanded: &HashSet<NodeKey>,
    reverse: bool,
    cap: usize,
) -> TreeRows {
    enum Frame {
        Enter {
            id: u32,
            key: NodeKey,
            depth: usize,
            kinds: Vec<DepKind>,
        },
        Exit(u32),
    }
    let mut result = TreeRows::default();
    if root as usize >= index.len() {
        return result;
    }
    let root_kind = if reverse {
        NodeKind::RevDirect
    } else {
        NodeKind::Input
    };
    let mut stack = vec![Frame::Enter {
        id: root,
        key: vec![(root, root_kind)],
        depth: 0,
        kinds: Vec::new(),
    }];
    let mut ancestors = HashSet::new();
    let mut descendants = 0;
    while let Some(frame) = stack.pop() {
        let (id, key, depth, kinds) = match frame {
            Frame::Exit(id) => {
                ancestors.remove(&id);
                continue;
            }
            Frame::Enter {
                id,
                key,
                depth,
                kinds,
            } => (id, key, depth, kinds),
        };
        if depth > 1 {
            if descendants >= cap.min(ROW_CAP) {
                result.materialization_limited = true;
                continue;
            }
            descendants += 1;
        }
        let cycle = ancestors.contains(&id);
        let children_count = if reverse {
            index.dependents_count(id)
        } else {
            index.packages[id as usize].dep_count()
        };
        let open = depth == 0 || expanded.contains(&key);
        result.rows.push(TreeRow {
            id,
            key: key.clone(),
            depth,
            kind: key.last().unwrap().1,
            kinds,
            children_count,
            is_section: false,
            transitive_depth: None,
            cycle,
        });
        if !cycle && open {
            ancestors.insert(id);
            stack.push(Frame::Exit(id));
            for (child, kinds) in children(index, id, reverse).into_iter().rev() {
                let mut path = key.clone();
                path.push((
                    child,
                    if reverse {
                        NodeKind::RevDirect
                    } else {
                        node_kind(kinds[0])
                    },
                ));
                stack.push(Frame::Enter {
                    id: child,
                    key: path,
                    depth: depth + 1,
                    kinds,
                });
            }
        }
    }
    result
}

fn filtered_rows(
    index: &Index,
    set: Option<&crate::relations::RelationSet>,
    visible: &[usize],
    reverse: bool,
) -> TreeRows {
    let Some(set) = set else {
        return TreeRows::default();
    };
    let rows = visible
        .iter()
        .filter_map(|&i| set.hits.get(i))
        .map(|hit| {
            let kind = if reverse {
                NodeKind::RevTrans
            } else {
                node_kind(hit.kinds[0])
            };
            TreeRow {
                id: hit.id,
                key: vec![(set.key.root, kind), (hit.id, kind)],
                depth: 1,
                kind,
                kinds: hit.kinds.clone(),
                children_count: if reverse {
                    index.dependents_count(hit.id)
                } else {
                    index.packages[hit.id as usize].dep_count()
                },
                is_section: false,
                transitive_depth: Some(hit.depth),
                cycle: false,
            }
        })
        .collect();
    TreeRows {
        rows,
        materialization_limited: false,
    }
}

/// Flatten an expanded tree, or use the complete worker-filtered closure.
pub fn dep_rows(index: &Index, state: &TreeState, root: u32, cap: usize) -> TreeRows {
    if !state.query.trim().is_empty() {
        return filtered_rows(index, state.relations.as_deref(), &state.visible, false);
    }
    flatten(index, root, &state.expanded, false, cap)
}

pub fn rev_rows(index: &Index, state: &RevState, root: u32, cap: usize) -> TreeRows {
    if !state.query.trim().is_empty() {
        return filtered_rows(index, state.relations.as_deref(), &state.visible, true);
    }
    let mut rows = flatten(index, root, &state.expanded, true, cap);
    if root as usize >= index.len() {
        return rows;
    }
    let transitive: Vec<_> = state
        .relations
        .as_ref()
        .map(|s| s.hits.iter().filter(|h| h.depth >= 2).collect())
        .unwrap_or_default();
    rows.rows.push(TreeRow {
        id: TRANS_SECTION,
        key: vec![(TRANS_SECTION, NodeKind::RevTrans)],
        depth: 0,
        kind: NodeKind::RevTrans,
        kinds: Vec::new(),
        children_count: transitive.len(),
        is_section: true,
        transitive_depth: None,
        cycle: false,
    });
    if state.trans_open {
        rows.rows.extend(transitive.into_iter().map(|hit| TreeRow {
            id: hit.id,
            key: vec![(root, NodeKind::RevTrans), (hit.id, NodeKind::RevTrans)],
            depth: 1,
            kind: NodeKind::RevTrans,
            kinds: hit.kinds.clone(),
            children_count: index.dependents_count(hit.id),
            is_section: false,
            transitive_depth: Some(hit.depth),
            cycle: false,
        }));
    }
    rows
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
    let Some(id) = (if reverse {
        app.rev_root_id()
    } else {
        app.selected_id()
    }) else {
        f.render_widget(
            ratatui::widgets::Paragraph::new("no package selected")
                .style(Style::default().fg(th.muted)),
            inner,
        );
        return;
    };

    let cached = if reverse {
        app.rev.rows.clone()
    } else {
        app.tree.rows.clone()
    };
    let rows = &cached.rows;
    if inner.height < 2 {
        return;
    }
    if !app.active_query().trim().is_empty() && rows.is_empty() {
        f.render_widget(
            ratatui::widgets::Paragraph::new(if app.relations_pending() {
                "Searching all declared relations…"
            } else if reverse {
                "No matching dependents"
            } else {
                "No matching dependencies"
            })
            .style(Style::default().fg(th.muted)),
            inner,
        );
        return;
    }
    let details_height = if inner.height >= 8 { 3 } else { 0 };
    let viewport = inner.height as usize - details_height - 1;
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
        Rect {
            height: viewport as u16,
            ..inner
        },
        &mut ListState::default().with_selected(Some(selected.min(viewport.saturating_sub(1)))),
    );
    let pkg = &index.packages[id as usize];
    let count = if reverse {
        index.dependents_count(id)
    } else {
        pkg.dep_count()
    };
    let relations = if reverse {
        app.rev.relations.as_ref()
    } else {
        app.tree.relations.as_ref()
    };
    let closure = relations
        .map(|s| s.hits.len().to_string())
        .unwrap_or_else(|| "computing…".into());
    let suffix = if cached.materialization_limited {
        " · expanded tree limit reached"
    } else {
        ""
    };
    let counts = if reverse {
        format!("{count} direct packages · {closure} reachable{suffix}")
    } else {
        format!(
            "{count} direct packages · {} typed relations · {closure} reachable{suffix}",
            pkg.relation_count()
        )
    };
    f.render_widget(
        ratatui::widgets::Paragraph::new(counts).style(Style::default().fg(th.muted)),
        Rect {
            y: inner.y + viewport as u16,
            height: 1,
            ..inner
        },
    );
    if details_height > 0 {
        if let Some(row) = rows.get(cursor).filter(|r| !r.is_section) {
            let p = &index.packages[row.id as usize];
            let text = format!(
                "{} {} · #{} · {}{}",
                p.name,
                p.version,
                p.id,
                if p.catalog {
                    "catalog"
                } else {
                    "private variant"
                },
                if row.cycle {
                    " · cycle, not expanded"
                } else {
                    ""
                }
            );
            f.render_widget(
                ratatui::widgets::Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: false }),
                Rect {
                    y: inner.y + viewport as u16 + 1,
                    height: details_height as u16,
                    ..inner
                },
            );
        }
    }
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
    let title = match app.rev_root_pkg() {
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
    let indent = row.depth.saturating_sub(if row.is_section { 1 } else { 0 });
    spans.push(Span::raw("  ".repeat(indent.min(12))));
    if indent > 12 {
        spans.push(Span::raw("… "));
    }

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
    let glyph = if row.cycle {
        "↻"
    } else if row.children_count > 0 {
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

    if let Some(depth) = row.transitive_depth {
        spans.push(Span::styled(
            format!(" d{depth}"),
            Style::default().fg(th.accent2),
        ));
    }
    for kind in &row.kinds {
        match kind {
            DepKind::Input => spans.push(badge("I", th.muted)),
            DepKind::Propagated => spans.push(badge("P", th.badge_p)),
            DepKind::Native => spans.push(badge("N", th.badge_n)),
        }
    }
    if row.cycle {
        spans.push(Span::raw(" cycle"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_lists_ignore_expanded_descendant_budget() {
        let count = 2103u32;
        let packages: Vec<_> = (0..count).map(|id| serde_json::json!({
            "id":id, "name":format!("p{id:05}"), "inputs":if id==0 { (1..count).collect::<Vec<_>>() } else { vec![] }
        })).collect();
        let doc = serde_json::from_value(
            serde_json::json!({"header":{"schema":4,"package_count":count},"packages":packages}),
        )
        .unwrap();
        let index = Index::from_doc(doc, 0).unwrap();
        let rows = dep_rows(&index, &TreeState::default(), 0, 2);
        assert_eq!(rows.rows.len(), count as usize);
        assert_eq!(rows.rows.last().unwrap().id, count - 1);
        assert!(!rows.materialization_limited);
    }
    #[test]
    fn diamond_is_depth_first_and_retains_both_branches_and_cycles() {
        let doc = serde_json::from_value(serde_json::json!({
            "header":{"schema":4,"package_count":4},"packages":[
                {"id":0,"name":"root","inputs":[1,2]}, {"id":1,"name":"a","inputs":[3]},
                {"id":2,"name":"b","inputs":[3]}, {"id":3,"name":"c","inputs":[0]}
            ]
        }))
        .unwrap();
        let index = Index::from_doc(doc, 0).unwrap();
        let mut state = TreeState::default();
        let k = NodeKind::Input;
        state.expanded.extend([
            vec![(0, k), (1, k)],
            vec![(0, k), (2, k)],
            vec![(0, k), (1, k), (3, k)],
            vec![(0, k), (2, k), (3, k)],
        ]);
        let result = dep_rows(&index, &state, 0, usize::MAX);
        let rows = &result.rows;
        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![0, 1, 3, 0, 2, 3, 0]
        );
        assert!(rows[3].cycle && rows[6].cycle);
        assert_ne!(rows[2].key, rows[5].key);
        assert!(!result.materialization_limited);
        assert!(dep_rows(&index, &state, 0, 1).materialization_limited);
        state.expanded.remove(&vec![(0, k), (1, k), (3, k)]);
        assert_eq!(
            dep_rows(&index, &state, 0, usize::MAX)
                .rows
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            vec![0, 1, 3, 2, 3, 0]
        );
    }
}
