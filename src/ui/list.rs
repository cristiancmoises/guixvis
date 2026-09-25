//! Search result list with fuzzy-match highlighting.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

use crate::app::App;
use crate::search::HighlightedHit;
use crate::theme::Theme;

/// Keep the cursor at least this many rows from the viewport edge.
const SCROLLOFF: usize = 3;

pub fn draw(f: &mut Frame, app: &mut App, th: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(th.border)
        .title(format!(
            " results {}",
            if app.search_pending() {
                "(Searching · previous results)".to_string()
            } else if app.query.is_empty() {
                "(hubs first · type to search)".to_string()
            } else {
                format!("({})", app.results.len())
            }
        ));
    let inner = block.inner(area);
    f.render_widget(block.clone(), area);
    if inner.height < 2 {
        return;
    }
    let viewport = inner.height as usize - 1;
    adjust_scroll(app, viewport);

    let items: Vec<ListItem> = app
        .results
        .iter()
        .skip(app.scroll)
        .take(viewport)
        .map(|h| ListItem::new(render_hit(app, th, h)))
        .collect();

    let list = List::new(items)
        .highlight_style(th.selected)
        .highlight_symbol("▶ ");
    f.render_stateful_widget(
        list,
        inner,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.cursor - app.scroll)),
    );
}

fn adjust_scroll(app: &mut App, viewport: usize) {
    if viewport == 0 || app.results.is_empty() {
        return;
    }
    let max_scroll = app.results.len().saturating_sub(viewport);
    if app.cursor < app.scroll + SCROLLOFF.min(viewport / 2) {
        app.scroll = app.cursor.saturating_sub(SCROLLOFF.min(viewport / 2));
    } else if app.cursor >= app.scroll + viewport.saturating_sub(SCROLLOFF.min(viewport / 2)) {
        app.scroll = (app.cursor + SCROLLOFF.min(viewport / 2) + 1).saturating_sub(viewport);
    }
    app.scroll = app.scroll.min(max_scroll);
}

fn render_hit(app: &App, th: &Theme, hit: &HighlightedHit) -> Line<'static> {
    let Some(index) = app.index.as_ref() else {
        return Line::raw("");
    };
    let p = &index.packages[hit.hit.id as usize];
    let name = p.name.as_ref();
    let version = p.version.as_ref();
    let synopsis: String = p.synopsis.chars().take(96).collect();

    let mut spans: Vec<Span> = Vec::new();
    // A name that the query did not match renders dimmed: the row is here
    // because of its synopsis, and the eye should not treat it as a name hit.
    let name_style = if !hit.hit.name_match && !app.query.is_empty() {
        Style::default().fg(th.muted)
    } else {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    };
    push_highlighted(&mut spans, name, &hit.name_ranges, name_style);
    if !version.is_empty() {
        spans.push(Span::styled(
            format!(" {version}"),
            Style::default().fg(th.muted),
        ));
    }
    // License chip.
    if let Some(lic) = p.licenses.first() {
        let extra = p.licenses.len().saturating_sub(1);
        let label = if extra > 0 {
            format!("{lic}+{extra}")
        } else {
            lic.to_string()
        };
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("·{label}"),
            Style::default().fg(th.badge_n),
        ));
    }
    // Facts that matter before you press Enter.
    let deps = p.dep_count();
    let dependents = index.dependents_count(p.id);
    spans.push(Span::styled(
        format!("  ⤵{deps} ⤴{dependents}"),
        Style::default().fg(th.muted),
    ));
    if !synopsis.is_empty() {
        spans.push(Span::raw("  "));
        push_highlighted(&mut spans, &synopsis, &hit.synopsis_ranges, th.matched);
    }
    Line::from(spans)
}

/// Render a string, highlighting the given char-offset ranges.
fn push_highlighted(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    ranges: &[(usize, usize)],
    style: Style,
) {
    if ranges.is_empty() {
        spans.push(Span::raw(text.to_string()));
        return;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut pos = 0usize;
    for (start, end) in ranges {
        let (s, e) = (*start, *end);
        if s < pos {
            continue;
        }
        // Plain text before this range.
        if let Some(&(start_byte, _)) = chars.get(pos) {
            let end_byte = chars.get(s).map(|c| c.0).unwrap_or(text.len());
            spans.push(Span::raw(text[start_byte..end_byte].to_string()));
        }
        // Highlighted range.
        if let Some(&(start_byte, _)) = chars.get(s) {
            let end_byte = chars.get(e).map(|c| c.0).unwrap_or(text.len());
            spans.push(Span::styled(text[start_byte..end_byte].to_string(), style));
        }
        pos = e;
    }
    if let Some(&(start_byte, _)) = chars.get(pos) {
        spans.push(Span::raw(text[start_byte..].to_string()));
    }
}
