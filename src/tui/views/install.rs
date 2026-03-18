use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::models::InstallResult;
use crate::tui::theme;

pub enum InstallState {
    CreatingPort,
    InstallingDriver,
    AddingPrinter,
    Complete(InstallResult),
}

pub fn render_install_view(
    f: &mut Frame,
    area: Rect,
    state: &InstallState,
    ip: &str,
    driver: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // title
            Constraint::Min(8),    // progress
            Constraint::Length(2),  // help bar
        ])
        .split(area);

    let title = Paragraph::new(format!("Installing printer at {ip}")).style(theme::TITLE);
    let title_block = Block::default().borders(Borders::BOTTOM);
    f.render_widget(title.block(title_block), chunks[0]);

    let failure_msg;
    let (step1, step2, step3, result_text) = match state {
        InstallState::CreatingPort => ("→ Creating TCP/IP port...", "  Installing driver...", "  Adding printer...", None),
        InstallState::InstallingDriver => ("✓ Port created", "→ Installing driver...", "  Adding printer...", None),
        InstallState::AddingPrinter => ("✓ Port created", "✓ Driver installed", "→ Adding printer...", None),
        InstallState::Complete(result) => {
            if result.success {
                ("✓ Port created", "✓ Driver installed", "✓ Printer added", Some(format!("\n  Printer '{}' is ready!", result.printer_name)))
            } else {
                let err = result.error.as_deref().unwrap_or("Unknown error");
                failure_msg = format!("✗ Failed: {err}");
                ("✓ Port created", "✓ Driver installed", failure_msg.as_str(), None)
            }
        }
    };

    let mut lines = vec![
        Line::from(format!("  {step1}")),
        Line::from(format!("  {step2}")),
        Line::from(format!("  {step3}")),
        Line::from(format!("  Driver: {driver}")),
    ];
    if let Some(rt) = result_text {
        lines.push(Line::from(rt));
    }

    let progress = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Progress "));
    f.render_widget(progress, chunks[1]);

    let help = match state {
        InstallState::Complete(_) => Line::from(vec![
            Span::styled("Esc", theme::HELP_KEY),
            Span::styled(" back  ", theme::HELP_TEXT),
            Span::styled("q", theme::HELP_KEY),
            Span::styled(" quit", theme::HELP_TEXT),
        ]),
        _ => Line::from(vec![
            Span::styled("Installing...", theme::DIM),
        ]),
    };
    f.render_widget(Paragraph::new(help), chunks[2]);
}
