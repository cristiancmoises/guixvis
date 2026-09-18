//! Hand-rolled theming: three palettes plus a NO_COLOR grayscale fallback.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent2: Color,
    pub badge_p: Color,
    pub badge_n: Color,
    pub matched: Style,
    pub selected: Style,
    pub border: Style,
    pub tab_active: Style,
    pub graph_node: Color,
    pub graph_edge: Color,
    pub graph_focus: Color,
}

pub const THEMES: [Theme; 3] = [DARK, ONE, LIGHT];

const DARK: Theme = Theme {
    name: "dark (default)",
    bg: Color::Rgb(10, 12, 16),
    fg: Color::Rgb(220, 224, 230),
    muted: Color::Rgb(120, 128, 140),
    accent: Color::Rgb(94, 196, 255),
    accent2: Color::Rgb(186, 120, 255),
    badge_p: Color::Rgb(255, 190, 60),
    badge_n: Color::Rgb(90, 210, 150),
    matched: Style::new()
        .fg(Color::Rgb(255, 214, 90))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(36, 46, 66))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(70, 78, 92)),
    tab_active: Style::new()
        .fg(Color::Rgb(94, 196, 255))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(94, 196, 255),
    graph_edge: Color::Rgb(70, 78, 92),
    graph_focus: Color::Rgb(255, 214, 90),
};

const ONE: Theme = Theme {
    name: "one",
    bg: Color::Rgb(20, 20, 24),
    fg: Color::Rgb(224, 226, 232),
    muted: Color::Rgb(130, 136, 150),
    accent: Color::Rgb(255, 120, 100),
    accent2: Color::Rgb(110, 190, 255),
    badge_p: Color::Rgb(255, 200, 80),
    badge_n: Color::Rgb(120, 220, 170),
    matched: Style::new()
        .fg(Color::Rgb(255, 200, 80))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(56, 44, 40))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(84, 80, 96)),
    tab_active: Style::new()
        .fg(Color::Rgb(255, 120, 100))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(255, 120, 100),
    graph_edge: Color::Rgb(84, 80, 96),
    graph_focus: Color::Rgb(255, 200, 80),
};

const LIGHT: Theme = Theme {
    name: "light",
    bg: Color::Rgb(244, 246, 250),
    fg: Color::Rgb(30, 34, 42),
    muted: Color::Rgb(110, 118, 134),
    accent: Color::Rgb(0, 110, 200),
    accent2: Color::Rgb(140, 60, 200),
    badge_p: Color::Rgb(170, 110, 0),
    badge_n: Color::Rgb(0, 130, 80),
    matched: Style::new()
        .fg(Color::Rgb(150, 90, 0))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(214, 224, 240))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(160, 168, 184)),
    tab_active: Style::new()
        .fg(Color::Rgb(0, 110, 200))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(0, 110, 200),
    graph_edge: Color::Rgb(180, 188, 202),
    graph_focus: Color::Rgb(150, 90, 0),
};

/// Grayscale fallback used when NO_COLOR is set.
const GRAY: Theme = Theme {
    name: "grayscale (NO_COLOR)",
    bg: Color::Rgb(0, 0, 0),
    fg: Color::Rgb(230, 230, 230),
    muted: Color::Rgb(150, 150, 150),
    accent: Color::Rgb(255, 255, 255),
    accent2: Color::Rgb(255, 255, 255),
    badge_p: Color::Rgb(230, 230, 230),
    badge_n: Color::Rgb(230, 230, 230),
    matched: Style::new()
        .fg(Color::Rgb(255, 255, 255))
        .add_modifier(Modifier::BOLD),
    selected: Style::new().add_modifier(Modifier::REVERSED),
    border: Style::new().fg(Color::Rgb(150, 150, 150)),
    tab_active: Style::new()
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::REVERSED),
    graph_node: Color::Rgb(230, 230, 230),
    graph_edge: Color::Rgb(150, 150, 150),
    graph_focus: Color::Rgb(255, 255, 255),
};

/// Pick a theme by index, honoring NO_COLOR.
pub fn theme(idx: usize) -> &'static Theme {
    if std::env::var_os("NO_COLOR").is_some() {
        return &GRAY;
    }
    &THEMES[idx % THEMES.len()]
}

pub fn theme_names() -> Vec<&'static str> {
    if std::env::var_os("NO_COLOR").is_some() {
        vec![GRAY.name]
    } else {
        THEMES.iter().map(|t| t.name).collect()
    }
}
