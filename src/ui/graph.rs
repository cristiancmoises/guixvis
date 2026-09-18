//! Graph view: force-directed dependency graph on a Canvas.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::canvas::{Canvas, Circle, Line as GLine};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Tab};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    // Build/refresh the layout when entering or after depth changes.
    if app.tab == Tab::Graph {
        app.ensure_graph();
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
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
            " graph of {} {} — depth {} — {} nodes, {} edges, ✂ {} hidden ",
            p.name, p.version, app.graph.depth, nodes, edges, app.graph.truncated
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

    // Node radii by degree, labels on wider terminals only.
    let label_mode = if area.width >= 140 {
        2 // selected + neighbors
    } else if area.width >= 100 {
        1 // selected only
    } else {
        0
    };

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(th.border),
        )
        .x_bounds([-1.6, 1.6])
        .y_bounds([-1.0, 1.0])
        .paint(|ctx| {
            for (a, b) in &app.graph.edges {
                let (x1, y1) = app.graph.pos[*a as usize];
                let (x2, y2) = app.graph.pos[*b as usize];
                ctx.draw(&GLine {
                    x1: x1 as f64,
                    y1: y1 as f64,
                    x2: x2 as f64,
                    y2: y2 as f64,
                    color: th.graph_edge,
                });
            }
            let selected = app.graph.selected;
            let neighbors = edges_from(&app.graph.edges, selected);
            for (i, id) in app.graph.nodes.iter().enumerate() {
                let p = &index.packages[*id as usize];
                let degree = index.dependents_count(*id) + p.dep_count();
                let radius = (0.03 + 0.014 * (1.0 + degree as f64).ln()).clamp(0.03, 0.09);
                let (x, y) = app.graph.pos[i];
                let color = if i == selected {
                    th.graph_focus
                } else if *id == app.graph.root {
                    th.accent
                } else {
                    th.graph_node
                };
                ctx.draw(&Circle {
                    x: x as f64,
                    y: y as f64,
                    radius,
                    color,
                });
                let is_selected = i == selected;
                let is_neighbor = neighbors.contains(&(i as u16));
                let show = match label_mode {
                    2 => is_selected || is_neighbor,
                    _ => is_selected,
                };
                if show || label_mode == 0 && i == selected {
                    let name: String = p.name.chars().take(24).collect();
                    let w = unicode_width::UnicodeWidthStr::width(name.as_str()) as f64;
                    ctx.print(x as f64 - w * 0.045, y as f64 + radius + 0.09, name);
                }
            }
        });
    f.render_widget(canvas, chunks[1]);

    let hint = match app.graph.selected_id() {
        Some(id) => index
            .packages
            .get(id as usize)
            .map(|p| {
                format!(
                    "selected: {} {} — Enter follow · +/− depth · g refocus · Tab cycle",
                    p.name, p.version
                )
            })
            .unwrap_or_default(),
        None => "Enter follow · +/− depth · g refocus".to_string(),
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(th.muted))),
        chunks[2],
    );
}

fn edges_from(edges: &[(u16, u16)], node: usize) -> Vec<u16> {
    edges
        .iter()
        .filter(|(a, b)| *a as usize == node || *b as usize == node)
        .flat_map(|(a, b)| [*a, *b])
        .filter(|n| *n as usize != node)
        .collect()
}
