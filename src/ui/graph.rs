//! Graph view: force-directed dependency graph on a Canvas.
//!
//! The picture is built for reading, not for looking pretty: node size grows
//! with fan-in/fan-out, color encodes BFS depth and the kind of dependency
//! that pulled the node in, and the selection gets a halo plus a brighter
//! neighbourhood. A legend line spells the encodings out, because a graph you
//! cannot decode is just noise.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Circle, Line as GLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::graph::EdgeMode;
use crate::index::DepKind;
use crate::theme::{self, Theme};

pub fn draw(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(if area.height >= 12 { 5 } else { 1 }),
    ])
    .split(area);
    let Some(index) = app.index.as_ref() else {
        f.render_widget(Paragraph::new("index loading…"), area);
        return;
    };
    let Some(root) = app
        .anchor
        .and_then(|_| index.packages.get(app.graph.root as usize))
    else {
        f.render_widget(Paragraph::new("no graph (select a package first)"), area);
        return;
    };
    let reached = app
        .graph
        .discovered_total
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("at least {}", app.graph.nodes.len() + app.graph.truncated));
    let edge_total = app
        .graph
        .edges_total
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unknown".into());
    let counts = format!(
        "{} {} · depth {} · {}/{} nodes · {}/{} edges{}",
        root.name,
        root.version,
        app.graph.depth,
        app.graph.nodes.len(),
        reached,
        app.graph.edges.len(),
        edge_total,
        if app.graph.edges_truncated > 0 {
            " · edge limit"
        } else {
            ""
        }
    );
    f.render_widget(
        Paragraph::new(counts).style(Style::default().fg(th.accent).add_modifier(Modifier::BOLD)),
        chunks[0],
    );
    let wide = area.width >= 110 && area.height >= 20;
    let hint = if wide {
        format!(
            "{} · +/− depth · / filter projected nodes",
            app.edge_mode.label()
        )
    } else {
        "List view · widen terminal for canvas · +/− depth".into()
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(th.muted)),
        chunks[1],
    );
    if app.graph_pending() {
        f.render_widget(
            Paragraph::new("Computing projection and layout…"),
            chunks[2],
        );
        return;
    }
    let panes = Layout::horizontal(if wide {
        [Constraint::Percentage(60), Constraint::Percentage(40)]
    } else {
        [Constraint::Length(0), Constraint::Min(1)]
    })
    .split(chunks[2]);
    if wide {
        draw_canvas(f, app, th, panes[0]);
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(th.border)
        .title(format!(
            " projected packages ({}/{}) ",
            app.graph_visible.len(),
            app.graph.nodes.len()
        ));
    let inner = block.inner(panes[1]);
    f.render_widget(block, panes[1]);
    let selected = app
        .graph_visible
        .iter()
        .position(|i| *i == app.graph.selected);
    let height = inner.height as usize;
    if let Some(pos) = selected {
        if pos < app.graph_scroll {
            app.graph_scroll = pos;
        }
        if height > 0 && pos >= app.graph_scroll + height {
            app.graph_scroll = pos + 1 - height;
        }
    }
    app.graph_scroll = app
        .graph_scroll
        .min(app.graph_visible.len().saturating_sub(height));
    if app.graph_visible.is_empty() {
        f.render_widget(Paragraph::new("No matching projected packages"), inner);
    } else {
        let items: Vec<ratatui::widgets::ListItem> = app
            .graph_visible
            .iter()
            .skip(app.graph_scroll)
            .take(height)
            .map(|&i| {
                let p = &index.packages[app.graph.nodes[i] as usize];
                ratatui::widgets::ListItem::new(format!(
                    "{} {} {} · d{} {}",
                    if p.id == app.graph.root {
                        "ROOT"
                    } else {
                        "    "
                    },
                    p.name,
                    p.version,
                    app.graph.depth_of[i],
                    app.graph.kinds_of[i]
                        .iter()
                        .map(|k| match k {
                            crate::index::DepKind::Input => "I",
                            crate::index::DepKind::Propagated => "P",
                            crate::index::DepKind::Native => "N",
                        })
                        .collect::<Vec<_>>()
                        .join("/")
                ))
            })
            .collect();
        f.render_stateful_widget(
            ratatui::widgets::List::new(items)
                .highlight_style(th.selected)
                .highlight_symbol("▶ "),
            inner,
            &mut ratatui::widgets::ListState::default()
                .with_selected(selected.map(|s| s.saturating_sub(app.graph_scroll))),
        );
    }
    let detail = app.graph.selected_id().map(|id| {
        let p=&index.packages[id as usize];
        format!("{} {}\n#{} · {} · {} dependencies · {} dependents\nEnter follow · Esc back · g anchor · [ / ] scroll details",
            p.name,p.version,p.id,if p.catalog { "catalog" } else { "private variant" },p.dep_count(),index.dependents_count(id))
    }).unwrap_or_else(|| "No selected package · / edit filter · Esc clear".into());
    f.render_widget(
        Paragraph::new(detail)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((app.graph_details_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(th.border),
            ),
        chunks[3],
    );
}

fn draw_canvas(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let Some(index) = &app.index else {
        return;
    };
    let selected = app.graph.selected;
    let neighbors = neighbors_of(&app.graph.edges, selected);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(th.border)
        .title(" dependency graph ");
    let inner = block.inner(area);
    let canvas = Canvas::default()
        .block(block)
        // Subcell dots keep converging edges from becoming a solid block.
        .marker(Marker::Braille)
        .x_bounds([-1.6, 1.6])
        .y_bounds([-1.0, 1.0])
        .paint(|ctx| {
            for &(a, b) in &app.graph.edges {
                let touching = a as usize == selected || b as usize == selected;
                let color = match app.edge_mode {
                    EdgeMode::None => continue,
                    EdgeMode::Focus if !touching => continue,
                    EdgeMode::Focus => theme::mix(th.graph_edge, th.accent, 0.5),
                    EdgeMode::All if touching => theme::mix(th.graph_edge, th.accent, 0.5),
                    EdgeMode::All => theme::mix(th.graph_edge, th.bg, 0.8),
                };
                let (x1, y1) = app.graph.pos[a as usize];
                let (x2, y2) = app.graph.pos[b as usize];
                ctx.draw(&GLine {
                    x1: x1 as f64,
                    y1: y1 as f64,
                    x2: x2 as f64,
                    y2: y2 as f64,
                    color,
                });
            }
            for (i, &id) in app.graph.nodes.iter().enumerate() {
                let (x, y) = app.graph.pos[i];
                let color = node_color(app, th, i, selected, &neighbors, !neighbors.is_empty());
                let radius = if i == selected {
                    0.035
                } else if id == app.graph.root {
                    0.026
                } else {
                    0.015
                };
                ctx.draw(&Circle {
                    x: x as f64,
                    y: y as f64,
                    radius,
                    color,
                });
            }
        });
    f.render_widget(canvas, area);
    let mut candidates = Vec::new();
    if selected < app.graph.nodes.len() {
        candidates.push(selected);
    }
    if !app.graph.nodes.is_empty() && selected != 0 {
        candidates.push(0);
    }
    if app.graph_labels {
        for &i in &app.graph_visible {
            if candidates.len() >= 6 {
                break;
            }
            if !candidates.contains(&i) {
                candidates.push(i);
            }
        }
    }
    let labels: Vec<_> = candidates
        .iter()
        .map(|&i| {
            let p = &index.packages[app.graph.nodes[i] as usize];
            (i, app.graph.pos[i], p.name.as_ref())
        })
        .collect();
    for (i, rect, text) in label_boxes(inner, &labels) {
        let style = Style::default().bg(th.bg).fg(if i == selected {
            th.graph_focus
        } else if i == 0 {
            th.accent
        } else {
            th.fg
        });
        f.render_widget(Block::default().style(style), rect);
        f.render_widget(Paragraph::new(text).style(style), rect);
    }
}

/// Position labels in terminal cells, avoiding intersections of the full text
/// boxes. Root and selection arrive first; details retain their complete names.
fn label_boxes(area: Rect, labels: &[(usize, (f32, f32), &str)]) -> Vec<(usize, Rect, String)> {
    use unicode_width::UnicodeWidthChar;
    let mut placed: Vec<(usize, Rect, String)> = Vec::new();
    if area.width == 0 || area.height == 0 {
        return placed;
    }
    for &(id, (x, y), name) in labels {
        let limit = area.width.min(28) as usize;
        let mut text = String::new();
        let mut width = 0usize;
        for c in name.chars() {
            let w = c.width().unwrap_or(0);
            if width + w > limit.saturating_sub(1) {
                text.push('…');
                width += 1;
                break;
            }
            text.push(c);
            width += w;
        }
        let width = width.max(1).min(area.width as usize) as u16;
        let cx = (((x + 1.6) / 3.2) * (area.width.saturating_sub(1)) as f32)
            .round()
            .clamp(0.0, area.width.saturating_sub(1) as f32) as u16;
        let cy = (((1.0 - y) / 2.0) * (area.height.saturating_sub(1)) as f32)
            .round()
            .clamp(0.0, area.height.saturating_sub(1) as f32) as u16;
        let left = cx.saturating_sub(width / 2).min(area.width - width) + area.x;
        let mut rows: Vec<_> = (0..area.height).collect();
        rows.sort_by_key(|row| row.abs_diff(cy));
        for row in rows {
            let rect = Rect {
                x: left,
                y: area.y + row,
                width,
                height: 1,
            };
            if placed.iter().all(|(_, other, _)| {
                rect.y != other.y
                    || rect.x.saturating_add(rect.width).saturating_add(1) <= other.x
                    || other.x.saturating_add(other.width).saturating_add(1) <= rect.x
            }) {
                placed.push((id, rect, text));
                break;
            }
        }
    }
    placed
}

/// Node color: depth sets the brightness, the dependency kind sets the hue,
/// and an active selection dims everything that is not a neighbour.
fn node_color(
    app: &App,
    th: &Theme,
    i: usize,
    selected: usize,
    neighbors: &[u16],
    focus: bool,
) -> Color {
    let depth = app.graph.depth_of.get(i).copied().unwrap_or(1);
    let base = match depth {
        0 => th.accent,
        1 => th.graph_node,
        2 => theme::mix(th.graph_node, th.bg, 0.38),
        _ => theme::mix(th.graph_node, th.bg, 0.64),
    };
    let mut color = match app.graph.kind_of.get(i).copied().flatten() {
        Some(DepKind::Propagated) => theme::mix(base, th.accent2, 0.55),
        Some(DepKind::Native) => theme::mix(base, th.badge_p, 0.45),
        _ => base,
    };
    if focus {
        if i == selected {
            color = th.graph_focus;
        } else if neighbors.contains(&(i as u16)) {
            color = theme::mix(color, th.accent, 0.4);
        } else {
            color = theme::mix(color, th.bg, 0.55);
        }
    } else if i == selected {
        color = th.graph_focus;
    }
    color
}

fn neighbors_of(edges: &[(u16, u16)], node: usize) -> Vec<u16> {
    edges
        .iter()
        .filter_map(|(a, b)| match (*a as usize == node, *b as usize == node) {
            (true, false) => Some(*b),
            (false, true) => Some(*a),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn label_boxes_do_not_overlap_or_escape_terminal_cells() {
        let area = Rect::new(3, 5, 40, 8);
        let labels = [
            (0, (0.0, 0.0), "very-long-root-package-name"),
            (1, (0.0, 0.0), "selected-dependency-name"),
            (2, (0.0, 0.0), "界界界界界界界界界界界界界界界"),
        ];
        let boxes = label_boxes(area, &labels);
        assert_eq!(boxes.len(), 3);
        for (i, (_, rect, text)) in boxes.iter().enumerate() {
            assert!(
                rect.x >= area.x
                    && rect.y >= area.y
                    && rect.right() <= area.right()
                    && rect.bottom() <= area.bottom()
            );
            assert!(unicode_width::UnicodeWidthStr::width(text.as_str()) <= rect.width as usize);
            for (_, other, _) in boxes.iter().skip(i + 1) {
                assert!(rect.y != other.y || rect.right() < other.x || other.right() < rect.x);
            }
        }
        assert!(label_boxes(Rect::new(0, 0, 0, 0), &labels).is_empty());
    }
}
