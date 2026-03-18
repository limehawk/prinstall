use ratatui::style::{Color, Modifier, Style};

// --- Typography ---
pub const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const HEADER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const SECTION_HEADER: Style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
pub const DIM: Style = Style::new().fg(Color::DarkGray);

// --- Selection ---
pub const SELECTED: Style = Style::new().fg(Color::Black).bg(Color::Cyan);

// --- Panel borders ---
pub const FOCUSED_BORDER: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const UNFOCUSED_BORDER: Style = Style::new().fg(Color::DarkGray);

// --- Printer status indicators (● ○ ◆ ✗) ---
pub const STATUS_READY: Style = Style::new().fg(Color::Green);
pub const STATUS_ERROR: Style = Style::new().fg(Color::Red);
pub const STATUS_OFFLINE: Style = Style::new().fg(Color::DarkGray);

// --- Printer source ---
pub const SOURCE_USB: Style = Style::new().fg(Color::Magenta);
pub const SOURCE_NETWORK: Style = Style::new().fg(Color::Cyan);

// --- Driver confidence badges (★ ● ○) ---
pub const EXACT_BADGE: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const FUZZY_BADGE: Style = Style::new().fg(Color::Yellow);

// --- Status bar messages ---
pub const STATUS_SUCCESS: Style = Style::new().fg(Color::Green);
pub const STATUS_ERROR_MSG: Style = Style::new().fg(Color::Red);
pub const STATUS_INFO: Style = Style::new().fg(Color::Cyan);

// --- Help bar ---
pub const HELP_KEY: Style = Style::new().fg(Color::Cyan);
pub const HELP_TEXT: Style = Style::new().fg(Color::DarkGray);
