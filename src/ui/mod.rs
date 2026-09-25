//! Rendering: root layout, header (tabs + search), status bar, and help.

pub mod detail;
pub mod graph;
pub mod help;
pub mod list;
pub mod tree;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, InputMode, Phase, Tab};
use crate::theme::{theme, Theme};

pub fn draw(f: &mut Frame, app: &mut App) {
    let th = theme(app.theme_idx);
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(th.bg)), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, app, th, chunks[0]);
    draw_body(f, app, th, chunks[1]);
    draw_status(f, app, th, chunks[2]);

    if app.help_open {
        help::draw(f, app, th, area);
    }
}

fn draw_header(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .split(area);

    // Tabs row.
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {}({}) ", tab.label(), tab.key());
        let style = if *tab == app.tab {
            th.tab_active
        } else {
            Style::default().fg(th.muted)
        };
        spans.push(Span::styled(label, style));
        if i + 1 < Tab::ALL.len() {
            spans.push(Span::styled("·", Style::default().fg(th.muted)));
        }
    }
    let displayed_pkg = if app.tab == Tab::RevDeps {
        app.rev_root_pkg()
    } else {
        app.selected_pkg()
    };
    if let Some(pkg) = displayed_pkg {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("{} {}", pkg.name, pkg.version),
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

    // Search row.
    let cursor = Span::styled(
        if app.mode == InputMode::Search {
            "▌"
        } else {
            ""
        },
        th.tab_active,
    );
    let label = match app.tab {
        Tab::Overview => "Search: ",
        Tab::Deps => "Filter dependencies: ",
        Tab::RevDeps => "Filter dependents: ",
        Tab::Graph => "Filter visible graph: ",
    };
    let query = app.active_query();
    let query_span = Span::styled(query, Style::default().fg(th.fg));
    let placeholder = if app.index.is_none() {
        "index loading — search will unlock…"
    } else if !query.is_empty() {
        ""
    } else if app.mode == InputMode::Navigate {
        "/ to search this tab"
    } else if matches!(app.tab, Tab::RevDeps | Tab::Deps) {
        "name + version · all direct and transitive relations"
    } else if app.tab == Tab::Graph {
        "name + version · projected nodes only"
    } else {
        "type a package name or synopsis"
    };
    let placeholder_span = Span::styled(placeholder, Style::default().fg(th.muted));
    let line = if query.is_empty() {
        Line::from(vec![Span::raw(label), cursor, placeholder_span])
    } else {
        Line::from(vec![Span::raw(label), query_span, cursor])
    };
    let block = Block::default()
        .title(if app.mode == InputMode::Search {
            " SEARCH · Enter/Esc finish "
        } else {
            " NAVIGATE · / search "
        })
        .borders(Borders::ALL)
        .border_style(th.border);
    f.render_widget(Paragraph::new(line).block(block), rows[1]);
}

fn draw_body(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    match app.tab {
        Tab::Overview => {
            let width = area.width.max(30);
            let left_w = (width * 55 / 100).min(area.width.saturating_sub(30));
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(0)])
                .split(area);
            list::draw(f, app, th, chunks[0]);
            detail::draw(f, app, th, chunks[1]);
        }
        Tab::Deps => tree::draw_deps(f, app, th, area),
        Tab::RevDeps => tree::draw_revs(f, app, th, area),
        Tab::Graph => graph::draw(f, app, th, area),
    }
}

fn draw_status(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let left = status_left(app, th);
    let right = Span::styled(
        if app.mode == InputMode::Search {
            "Enter done · F1 help · Ctrl+C quit".to_string()
        } else {
            "/ search · ? help · q quit · R rebuild".to_string()
        },
        Style::default().fg(th.muted),
    );
    let mut spans = vec![left];
    let left_len: usize = spans.iter().map(|s| s.width()).sum();
    let right_len = right.width();
    let gap = area.width as usize;
    if gap > left_len + right_len + 4 {
        spans.push(Span::raw(" ".repeat(gap - left_len - right_len)));
    }
    spans.push(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn status_left(app: &App, th: &Theme) -> Span<'static> {
    match &app.phase {
        Phase::Loading { done, total } => {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = frames[(app.tick as usize / 3) % frames.len()];
            let progress = if *total > 0 {
                format!("{done}/{total}")
            } else {
                "starting".to_string()
            };
            Span::styled(
                format!(
                    "{spinner} indexing Guix packages… {progress} (first run can take minutes)"
                ),
                Style::default().fg(th.accent),
            )
        }
        Phase::Ready { fresh, unkeyed } => {
            let Some(index) = app.index.as_ref() else {
                return Span::raw("");
            };
            let commit = if index.guix_commit.is_empty() {
                "unknown commit".to_string()
            } else {
                index.guix_commit.chars().take(7).collect::<String>()
            };
            let mut text = format!("{} pkgs", index.len());
            text.push_str(&format!(
                " · cache {}",
                if *fresh { "fresh" } else { "rebuilt" }
            ));
            if *unkeyed {
                text.push_str(" · origin unverified");
            }
            if !index.is_complete() {
                text.push_str(&format!(
                    " · incomplete ({} diagnostics)",
                    index.diagnostics.len()
                ));
            }
            text.push_str(&format!(" · {commit}"));
            let count = app.results.len();
            if app.tab == Tab::Overview && app.search_pending() {
                Span::styled(
                    format!("Searching · previous results · {text}"),
                    Style::default().fg(th.accent),
                )
            } else if app.tab == Tab::Overview && count > 0 && !app.query.is_empty() {
                Span::styled(
                    format!("{count} matches · {text}"),
                    Style::default().fg(th.accent),
                )
            } else {
                Span::styled(text, Style::default().fg(th.muted))
            }
        }
        Phase::Failed { msg } => Span::styled(
            format!("✗ {msg} — press R to retry"),
            Style::default().fg(th.graph_focus),
        ),
    }
}
