use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::models::{InstallDetail, PrinterOpResult};
use crate::tui::theme;

/// Render install progress in the detail pane.
///
/// `step` — 0=port, 1=driver, 2=printer, 3=all done (used only when `complete` is true).
/// `error` — present when current step failed.
/// `complete` — true when install has finished (success or failure).
/// `result` — the final PrinterOpResult, present only when `complete` is true.
#[allow(clippy::too_many_arguments)]
pub fn render_install_progress(
    f: &mut Frame,
    area: Rect,
    step: usize,
    error: Option<&str>,
    ip: &str,
    driver: &str,
    complete: bool,
    result: Option<&PrinterOpResult>,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Install ")
        .border_style(theme::FOCUSED_BORDER);

    // Step symbol helpers
    let done = |s: usize| -> Span<'static> {
        if complete || s < step {
            Span::styled("✓ ", theme::STATUS_SUCCESS)
        } else if s == step {
            if error.is_some() {
                Span::styled("✗ ", theme::STATUS_ERROR_MSG)
            } else {
                Span::styled("→ ", theme::STATUS_INFO)
            }
        } else {
            Span::styled("· ", theme::DIM)
        }
    };

    let step_label = |s: usize, label: &'static str| -> Line<'static> {
        Line::from(vec![Span::raw("  "), done(s), Span::raw(label)])
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  Installing: ", theme::HEADER),
            Span::raw(ip.to_string()),
        ]),
        Line::from(vec![
            Span::styled("  Driver:     ", theme::HEADER),
            Span::raw(driver.to_string()),
        ]),
        Line::from(""),
        step_label(0, "Creating TCP/IP port"),
        step_label(1, "Installing driver"),
        step_label(2, "Adding printer"),
    ];

    if let Some(err) = error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Error: {err}"),
            theme::STATUS_ERROR_MSG,
        )));
    }

    if complete {
        lines.push(Line::from(""));
        if let Some(r) = result {
            if r.success {
                let name = r
                    .detail_as::<InstallDetail>()
                    .map(|d| d.printer_name)
                    .unwrap_or_else(|| "printer".to_string());
                lines.push(Line::from(Span::styled(
                    format!("  '{name}' is ready!"),
                    theme::STATUS_SUCCESS,
                )));
            } else {
                let err = r.error.as_deref().unwrap_or("unknown error");
                lines.push(Line::from(Span::styled(
                    format!("  Failed: {err}"),
                    theme::STATUS_ERROR_MSG,
                )));
            }
        }
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}
