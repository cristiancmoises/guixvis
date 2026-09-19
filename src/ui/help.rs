//! Help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::theme::Theme;

pub fn draw(f: &mut Frame, _app: &App, th: &Theme, area: Rect) {
    let width = area.width.min(76);
    let height = area.height.min(26);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);

    let keys: &[(&str, &str)] = &[
        ("/", "focus search"),
        ("Esc", "clear search / back"),
        ("Tab / Shift+Tab", "cycle tabs"),
        ("1–4", "jump to tab"),
        ("↑↓ / jk", "move selection"),
        ("PgUp / PgDn", "page"),
        ("g / G", "top / bottom (graph: g refocus)"),
        ("Enter", "expand/collapse (trees) · follow (graph)"),
        ("d / r / v", "dependencies / reverse / graph"),
        ("h / l", "collapse / expand tree node"),
        ("+ / −", "graph depth"),
        ("e", "graph edges (all / focus / none)"),
        ("l", "graph labels on / off"),
        ("o", "open homepage in browser"),
        ("T", "cycle theme (8 palettes)"),
        ("R", "rebuild index"),
        ("q / Ctrl+C", "quit"),
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(popup);

    f.render_widget(
        Paragraph::new(Span::styled(
            " guixvis — keymap ",
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(th.border),
        ),
        chunks[0],
    );

    let mut lines: Vec<Line> = Vec::new();
    for (key, desc) in keys {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>13}  ", key),
                Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(*desc),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(th.border),
        ),
        chunks[1],
    );
}
