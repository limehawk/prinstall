use ratatui::prelude::*;

/// Layout mode based on terminal width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutMode {
    /// >= 100 cols: side-by-side panels
    Wide,
    /// 60-99 cols: stacked panels
    Stacked,
    /// < 60 cols: single panel, detail as overlay
    Narrow,
}

impl LayoutMode {
    pub fn from_width(width: u16) -> Self {
        if width >= 100 {
            Self::Wide
        } else if width >= 60 {
            Self::Stacked
        } else {
            Self::Narrow
        }
    }
}

/// Compute the main layout areas: header, panels, status bar.
pub fn main_layout(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(5),   // panels
            Constraint::Length(1), // status bar
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
}

/// Split the panel area into printer list and detail pane.
/// Returns `(printer_list_area, detail_area_or_none)`.
pub fn panel_layout(area: Rect, mode: LayoutMode) -> (Rect, Option<Rect>) {
    match mode {
        LayoutMode::Wide => {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(35),
                    Constraint::Percentage(65),
                ])
                .split(area);
            (chunks[0], Some(chunks[1]))
        }
        LayoutMode::Stacked => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Percentage(60),
                ])
                .split(area);
            (chunks[0], Some(chunks[1]))
        }
        LayoutMode::Narrow => (area, None),
    }
}
