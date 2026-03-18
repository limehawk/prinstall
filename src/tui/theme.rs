use ratatui::style::{Color, Modifier, Style};

pub const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const HEADER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const SELECTED: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
pub const STATUS_READY: Style = Style::new().fg(Color::Green);
pub const STATUS_ERROR: Style = Style::new().fg(Color::Red);
pub const STATUS_OFFLINE: Style = Style::new().fg(Color::DarkGray);
pub const SECTION_HEADER: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
pub const EXACT_BADGE: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const FUZZY_BADGE: Style = Style::new().fg(Color::Yellow);
pub const HELP_KEY: Style = Style::new().fg(Color::Cyan);
pub const HELP_TEXT: Style = Style::new().fg(Color::DarkGray);
pub const DIM: Style = Style::new().fg(Color::DarkGray);
