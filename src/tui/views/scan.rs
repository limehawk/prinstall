use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::models::{Printer, PrinterStatus};
use crate::tui::theme;

pub fn render_scan_view(
    f: &mut Frame,
    area: Rect,
    printers: &[Printer],
    selected: usize,
    scanning: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(5),   // table
            Constraint::Length(2), // help bar
        ])
        .split(area);

    // Title
    let title = if scanning {
        Paragraph::new("Scanning network...").style(theme::TITLE)
    } else {
        Paragraph::new(format!("Found {} printer(s)", printers.len())).style(theme::TITLE)
    };
    let title_block = Block::default().borders(Borders::BOTTOM);
    f.render_widget(title.block(title_block), chunks[0]);

    // Printer table
    let header = Row::new(vec!["IP", "Model", "Status"]).style(theme::HEADER);

    let rows: Vec<Row> = printers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let status_style = match p.status {
                PrinterStatus::Ready => theme::STATUS_READY,
                PrinterStatus::Error => theme::STATUS_ERROR,
                PrinterStatus::Offline => theme::STATUS_OFFLINE,
                PrinterStatus::Unknown => theme::DIM,
            };
            let row = Row::new(vec![
                Cell::from(p.ip.clone()),
                Cell::from(p.model.as_deref().unwrap_or("Unknown").to_string()),
                Cell::from(p.status.to_string()).style(status_style),
            ]);
            if i == selected {
                row.style(theme::SELECTED)
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Min(30),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Printers "));

    f.render_widget(table, chunks[1]);

    // Help bar
    let help = Line::from(vec![
        Span::styled("↑↓", theme::HELP_KEY),
        Span::styled(" navigate  ", theme::HELP_TEXT),
        Span::styled("Enter", theme::HELP_KEY),
        Span::styled(" drivers  ", theme::HELP_TEXT),
        Span::styled("i", theme::HELP_KEY),
        Span::styled(" identify  ", theme::HELP_TEXT),
        Span::styled("s", theme::HELP_KEY),
        Span::styled(" rescan  ", theme::HELP_TEXT),
        Span::styled("q", theme::HELP_KEY),
        Span::styled(" quit", theme::HELP_TEXT),
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);
}
