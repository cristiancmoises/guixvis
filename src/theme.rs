//! Hand-rolled theming: eight palettes plus a NO_COLOR grayscale fallback.
//! Cycle with `T` (forward) and `Shift+T` (backward); the active theme name
//! is shown in the status bar.

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

pub const THEMES: [Theme; 8] = [
    DARK,
    ONE,
    LIGHT,
    DRACULA,
    NORD,
    GRUVBOX,
    TOKYO_NIGHT,
    CATPPUCCIN,
];

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

const DRACULA: Theme = Theme {
    name: "dracula",
    bg: Color::Rgb(40, 42, 54),
    fg: Color::Rgb(248, 248, 242),
    muted: Color::Rgb(98, 114, 164),
    accent: Color::Rgb(139, 233, 253),
    accent2: Color::Rgb(255, 121, 198),
    badge_p: Color::Rgb(255, 184, 108),
    badge_n: Color::Rgb(80, 250, 123),
    matched: Style::new()
        .fg(Color::Rgb(241, 250, 140))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(68, 71, 90))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(68, 71, 90)),
    tab_active: Style::new()
        .fg(Color::Rgb(139, 233, 253))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(139, 233, 253),
    graph_edge: Color::Rgb(68, 71, 90),
    graph_focus: Color::Rgb(241, 250, 140),
};

const NORD: Theme = Theme {
    name: "nord",
    bg: Color::Rgb(46, 52, 64),
    fg: Color::Rgb(216, 222, 233),
    muted: Color::Rgb(129, 161, 193),
    accent: Color::Rgb(136, 192, 208),
    accent2: Color::Rgb(180, 142, 173),
    badge_p: Color::Rgb(235, 203, 139),
    badge_n: Color::Rgb(163, 190, 140),
    matched: Style::new()
        .fg(Color::Rgb(235, 203, 139))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(59, 66, 82))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(76, 86, 106)),
    tab_active: Style::new()
        .fg(Color::Rgb(136, 192, 208))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(136, 192, 208),
    graph_edge: Color::Rgb(76, 86, 106),
    graph_focus: Color::Rgb(235, 203, 139),
};

const GRUVBOX: Theme = Theme {
    name: "gruvbox-dark",
    bg: Color::Rgb(40, 40, 40),
    fg: Color::Rgb(235, 219, 178),
    muted: Color::Rgb(146, 131, 116),
    accent: Color::Rgb(131, 165, 152),
    accent2: Color::Rgb(211, 134, 155),
    badge_p: Color::Rgb(250, 189, 47),
    badge_n: Color::Rgb(184, 187, 38),
    matched: Style::new()
        .fg(Color::Rgb(250, 189, 47))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(60, 56, 54))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(80, 73, 69)),
    tab_active: Style::new()
        .fg(Color::Rgb(131, 165, 152))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(131, 165, 152),
    graph_edge: Color::Rgb(80, 73, 69),
    graph_focus: Color::Rgb(250, 189, 47),
};

const TOKYO_NIGHT: Theme = Theme {
    name: "tokyo-night",
    bg: Color::Rgb(26, 27, 38),
    fg: Color::Rgb(192, 202, 245),
    muted: Color::Rgb(86, 95, 137),
    accent: Color::Rgb(122, 162, 247),
    accent2: Color::Rgb(187, 154, 247),
    badge_p: Color::Rgb(224, 175, 104),
    badge_n: Color::Rgb(158, 206, 106),
    matched: Style::new()
        .fg(Color::Rgb(224, 175, 104))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(36, 40, 59))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(41, 46, 66)),
    tab_active: Style::new()
        .fg(Color::Rgb(122, 162, 247))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(122, 162, 247),
    graph_edge: Color::Rgb(41, 46, 66),
    graph_focus: Color::Rgb(224, 175, 104),
};

const CATPPUCCIN: Theme = Theme {
    name: "catppuccin-mocha",
    bg: Color::Rgb(30, 30, 46),
    fg: Color::Rgb(205, 214, 244),
    muted: Color::Rgb(108, 112, 134),
    accent: Color::Rgb(137, 180, 250),
    accent2: Color::Rgb(203, 166, 247),
    badge_p: Color::Rgb(249, 226, 175),
    badge_n: Color::Rgb(166, 227, 161),
    matched: Style::new()
        .fg(Color::Rgb(249, 226, 175))
        .add_modifier(Modifier::BOLD),
    selected: Style::new()
        .bg(Color::Rgb(49, 50, 68))
        .add_modifier(Modifier::BOLD),
    border: Style::new().fg(Color::Rgb(69, 71, 90)),
    tab_active: Style::new()
        .fg(Color::Rgb(137, 180, 250))
        .add_modifier(Modifier::BOLD),
    graph_node: Color::Rgb(137, 180, 250),
    graph_edge: Color::Rgb(69, 71, 90),
    graph_focus: Color::Rgb(249, 226, 175),
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

/// Blend two colors: `t == 0.0` keeps `a`, `t == 1.0` yields `b`.
///
/// Colors that are not RGB (the terminal's 16 basic colors, for instance)
/// are returned unchanged; there is nothing meaningful to interpolate.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return if t < 0.5 { a } else { b };
    };
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
}

pub fn theme_names() -> Vec<&'static str> {
    if std::env::var_os("NO_COLOR").is_some() {
        vec![GRAY.name]
    } else {
        THEMES.iter().map(|t| t.name).collect()
    }
}
