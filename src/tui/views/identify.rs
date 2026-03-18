use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::models::Printer;
use crate::tui::theme;

pub fn render_identify_view(f: &mut Frame, area: Rect, printer: &Printer) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    let title = Paragraph::new(format!("Printer at {}", printer.ip)).style(theme::TITLE);
    let title_block = Block::default().borders(Borders::BOTTOM);
    f.render_widget(title.block(title_block), chunks[0]);

    let details = vec![
        Line::from(vec![
            Span::styled("  Model:  ", theme::HEADER),
            Span::raw(printer.model.as_deref().unwrap_or("Unknown")),
        ]),
        Line::from(vec![
            Span::styled("  Serial: ", theme::HEADER),
            Span::raw(printer.serial.as_deref().unwrap_or("N/A")),
        ]),
        Line::from(vec![
            Span::styled("  Status: ", theme::HEADER),
            Span::raw(printer.status.to_string()),
        ]),
    ];

    let detail_widget = Paragraph::new(details)
        .block(Block::default().borders(Borders::ALL).title(" Details "));
    f.render_widget(detail_widget, chunks[1]);

    let help = Line::from(vec![
        Span::styled("d", theme::HELP_KEY),
        Span::styled(" drivers  ", theme::HELP_TEXT),
        Span::styled("Esc", theme::HELP_KEY),
        Span::styled(" back  ", theme::HELP_TEXT),
        Span::styled("q", theme::HELP_KEY),
        Span::styled(" quit", theme::HELP_TEXT),
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);
}
