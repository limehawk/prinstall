use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Check if a key event matches a key code (no modifiers).
pub fn key(event: KeyEvent, code: KeyCode) -> bool {
    event.code == code && event.modifiers == KeyModifiers::NONE
}

/// Check if a key event matches a character.
pub fn char(event: KeyEvent, c: char) -> bool {
    key(event, KeyCode::Char(c))
}

/// Check if a key event is shift+tab.
pub fn shift_tab(event: KeyEvent) -> bool {
    event.code == KeyCode::BackTab
}
