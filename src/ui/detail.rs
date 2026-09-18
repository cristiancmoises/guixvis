//! Package detail pane (Overview tab, right side).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::index::{DepKind, Package};
use crate::theme::Theme;

pub fn draw(f: &mut Frame, app: &App, th: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(th.border)
        .title(" package ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(pkg) = app.selected_pkg() else {
        let hint = if app.index.is_none() {
            "Index is loading…"
        } else {
            "Select a package to see details."
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().fg(th.muted)),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Header: name, version, licenses.
    let mut header: Vec<Span> = vec![Span::styled(
        pkg.name.as_ref(),
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
    )];
    if !pkg.version.is_empty() {
        header.push(Span::styled(
            format!(" {}", pkg.version),
            Style::default().fg(th.fg),
        ));
    }
    for lic in pkg.licenses.iter().take(3) {
        header.push(Span::raw("  "));
        header.push(Span::styled(
            format!("·{}·", lic),
            Style::default().fg(th.accent2),
        ));
    }
    lines.push(Line::from(header));
    lines.push(Line::raw(""));

    // Synopsis.
    if !pkg.synopsis.is_empty() {
        lines.push(Line::from(Span::styled(
            pkg.synopsis.as_ref(),
            Style::default().add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::raw(""));
    }

    // Description.
    if !pkg.description.is_empty() {
        lines.push(Line::from(pkg.description.as_ref()));
        lines.push(Line::raw(""));
    }

    // Metadata.
    if !pkg.homepage.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Home: ", Style::default().fg(th.muted)),
            Span::styled(
                pkg.homepage.as_ref(),
                Style::default()
                    .fg(th.accent)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ]));
    }
    if !pkg.file.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("File: ", Style::default().fg(th.muted)),
            Span::styled(
                format!("{}:{}", pkg.file, pkg.line),
                Style::default().fg(th.fg),
            ),
        ]));
    }

    // Dependency summary.
    let counts = dep_counts(app, pkg);
    lines.push(Line::from(vec![
        Span::styled("Deps: ", Style::default().fg(th.muted)),
        Span::styled(
            format!(
                "{} total ({} direct · {} propagated · {} native)",
                counts.0, counts.1, counts.2, counts.3
            ),
            Style::default().fg(th.fg),
        ),
    ]));
    let dependents = app
        .index
        .as_ref()
        .map(|i| i.dependents_count(pkg.id))
        .unwrap_or(0);
    lines.push(Line::from(vec![
        Span::styled("Dependents: ", Style::default().fg(th.muted)),
        Span::styled(format!("⤴ {dependents}"), Style::default().fg(th.badge_p)),
    ]));
    let neighbors = app
        .index
        .as_ref()
        .map(|i| i.module_neighbors(pkg.id).len().saturating_sub(1))
        .unwrap_or(0);
    lines.push(Line::from(vec![
        Span::styled("Same module: ", Style::default().fg(th.muted)),
        Span::styled(format!("{neighbors}"), Style::default().fg(th.fg)),
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "d deps · r reverse · v graph · o open home",
        Style::default().fg(th.muted),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn dep_counts(app: &App, pkg: &Package) -> (usize, usize, usize, usize) {
    let (mut total, mut inp, mut prop, mut nat) = (0, 0, 0, 0);
    let _ = app;
    for (_, kind) in pkg.deps() {
        total += 1;
        match kind {
            DepKind::Input => inp += 1,
            DepKind::Propagated => prop += 1,
            DepKind::Native => nat += 1,
        }
    }
    (total, inp, prop, nat)
}
