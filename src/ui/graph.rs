//! Graph view: force-directed dependency graph on a Canvas.
//!
//! The picture is built for reading, not for looking pretty: node size grows
//! with fan-in/fan-out, color encodes BFS depth and the kind of dependency
//! that pulled the node in, and the selection gets a halo plus a brighter
//! neighbourhood. A legend line spells the encodings out, because a graph you
//! cannot decode is just noise.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Line as GLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Tab};
use crate::index::DepKind;
use crate::theme::{self, Theme};

pub fn draw(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    // Build/refresh the layout when entering or after depth changes.
    if app.tab == Tab::Graph {
        app.ensure_graph();
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    let Some(index) = app.index.as_ref() else {
        f.render_widget(
            Paragraph::new("index loading…").style(Style::default().fg(th.muted)),
            area,
        );
        return;
    };
    let root_pkg = index.packages.get(app.graph.root as usize);
    let (nodes, edges) = (app.graph.nodes.len(), app.graph.edges.len());
    let header = match root_pkg {
        Some(p) => format!(
            " graph of {} {} — depth {} — {} nodes · {} edges · layout {:.0} ms{} ",
            p.name,
            p.version,
            app.graph.depth,
            nodes,
            edges,
            app.graph.layout_ms,
            if app.graph.truncated > 0 {
                format!(" · ✂ {} hidden", app.graph.truncated)
            } else {
                String::new()
            }
        ),
        None => " graph ".to_string(),
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            header,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    if app.graph.nodes.is_empty() {
        f.render_widget(
            Paragraph::new("no graph (select a package first)")
                .style(Style::default().fg(th.muted)),
            chunks[1],
        );
        return;
    }

    // Which dependency kinds actually occur here? Only advertise what exists.
    let mut has_propagated = false;
    let mut has_native = false;
    for kind in app.graph.kind_of.iter().flatten() {
        match kind {
            DepKind::Propagated => has_propagated = true,
            DepKind::Native => has_native = true,
            DepKind::Input => {}
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(legend(th, has_propagated, has_native))),
        chunks[1],
    );

    // Labels: the selection and the root always, then the biggest hubs that
    // fit on this terminal width.
    let label_budget = if area.width >= 150 {
        14
    } else if area.width >= 110 {
        7
    } else if area.width >= 84 {
        3
    } else {
        0
    };
    let selected = app.graph.selected;
    let mut label_nodes: Vec<usize> = Vec::new();
    if label_budget > 0 {
        label_nodes.push(selected.min(nodes - 1));
        if app.graph.root as usize != selected {
            label_nodes.push(0);
        }
        if app.graph.depth_of.len() == nodes {
            let mut by_degree: Vec<(usize, usize)> = (0..nodes)
                .map(|i| {
                    let id = app.graph.nodes[i] as usize;
                    (
                        i,
                        index.packages.get(id).map_or(0, |p| p.dep_count())
                            + index.dependents_count(app.graph.nodes[i]),
                    )
                })
                .filter(|(i, _)| !label_nodes.contains(i))
                .collect();
            by_degree.sort_by(|a, b| b.1.cmp(&a.1));
            label_nodes.extend(by_degree.into_iter().take(label_budget).map(|(i, _)| i));
        }
    }

    // Neighbours of the selection: brighter nodes, brighter edges.
    let neighbors = neighbors_of(&app.graph.edges, selected);
    let focus = !neighbors.is_empty();

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(th.border),
        )
        .marker(Marker::HalfBlock)
        .x_bounds([-1.6, 1.6])
        .y_bounds([-1.0, 1.0])
        .paint(|ctx| {
            for (a, b) in &app.graph.edges {
                let (x1, y1) = app.graph.pos[*a as usize];
                let (x2, y2) = app.graph.pos[*b as usize];
                let touching = *a as usize == selected || *b as usize == selected;
                let color = if touching {
                    theme::mix(th.graph_edge, th.accent, 0.75)
                } else if focus {
                    theme::mix(th.graph_edge, th.bg, 0.35)
                } else {
                    th.graph_edge
                };
                ctx.draw(&GLine {
                    x1: x1 as f64,
                    y1: y1 as f64,
                    x2: x2 as f64,
                    y2: y2 as f64,
                    color,
                });
            }

            for (i, id) in app.graph.nodes.iter().enumerate() {
                let p = &index.packages[*id as usize];
                let degree = index.dependents_count(*id) + p.dep_count();
                let radius = (0.035 + 0.016 * (1.0 + degree as f64).ln()).clamp(0.035, 0.115);
                let (x, y) = app.graph.pos[i];
                let color = node_color(app, th, i, selected, &neighbors, focus);

                if i == selected {
                    // Halo: a soft disc under the marker so the cursor is
                    // findable even in a dense cluster.
                    ctx.draw(&Circle {
                        x: x as f64,
                        y: y as f64,
                        radius: radius * 2.1,
                        color: theme::mix(th.graph_focus, th.bg, 0.72),
                    });
                }
                ctx.draw(&Circle {
                    x: x as f64,
                    y: y as f64,
                    radius,
                    color,
                });
            }

            // Labels last so they sit on top of the markers.
            let mut placed: Vec<(f64, f64)> = Vec::new();
            for i in label_nodes.iter().copied() {
                let Some(id) = app.graph.nodes.get(i) else {
                    continue;
                };
                let p = &index.packages[*id as usize];
                let (x, y) = app.graph.pos[i];
                if placed
                    .iter()
                    .any(|(px, py)| (px - x as f64).abs() < 0.22 && (py - y as f64).abs() < 0.14)
                {
                    continue;
                }
                placed.push((x as f64, y as f64));
                let name: String = p.name.chars().take(26).collect();
                let w = unicode_width::UnicodeWidthStr::width(name.as_str()) as f64;
                let dy = if y > 0.55 { -0.16 } else { 0.13 };
                ctx.print(
                    x as f64 - w * 0.042,
                    y as f64 + dy,
                    Span::styled(
                        name,
                        Style::default().fg(if i == selected {
                            th.graph_focus
                        } else if i == 0 {
                            th.accent
                        } else {
                            theme::mix(th.fg, th.bg, 0.25)
                        }),
                    ),
                );
            }
        });
    f.render_widget(canvas, chunks[2]);

    let hint = match app.graph.selected_id() {
        Some(id) => index.packages.get(id as usize).map_or_else(String::new, |p| {
            let deg = index.dependents_count(id);
            format!(
                " {} {} · {} deps · {} dependents — Enter follow · +/− depth · g refocus · Tab cycle ",
                p.name,
                p.version,
                p.dep_count(),
                deg
            )
        }),
        None => " Enter follow · +/− depth · g refocus".to_string(),
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(th.muted))),
        chunks[3],
    );
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

/// One-line decoding aid for the colors above.
fn legend(th: &Theme, propagated: bool, native: bool) -> Vec<Span<'static>> {
    let dot = |color: Color, label: &str| {
        vec![
            Span::styled("● ", Style::default().fg(color)),
            Span::styled(format!("{label}  "), Style::default().fg(th.muted)),
        ]
    };
    let mut spans = vec![Span::styled(" ", Style::default())];
    spans.extend(dot(th.accent, "root"));
    spans.extend(dot(th.graph_node, "direct dep"));
    spans.extend(dot(theme::mix(th.graph_node, th.bg, 0.64), "deep dep"));
    if propagated {
        spans.extend(dot(
            theme::mix(th.graph_node, th.accent2, 0.55),
            "propagated",
        ));
    }
    if native {
        spans.extend(dot(theme::mix(th.graph_node, th.badge_p, 0.45), "native"));
    }
    spans.extend(dot(th.graph_focus, "selected"));
    spans.push(Span::styled(
        "· size = fan-in + fan-out",
        Style::default().fg(th.muted),
    ));
    spans
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
