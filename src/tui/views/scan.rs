use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::models::{Printer, PrinterSource, PrinterStatus};
use crate::tui::theme;

pub fn render_printer_list(
    f: &mut Frame,
    area: Rect,
    printers: &[Printer],
    scanning: bool,
    list_state: &mut ListState,
    focused: bool,
) {
    let border_style = if focused {
        theme::FOCUSED_BORDER
    } else {
        theme::UNFOCUSED_BORDER
    };

    let title = if scanning {
        " Printers (scanning...) "
    } else {
        " Printers "
    };

    let items: Vec<ListItem> = printers
        .iter()
        .map(|p| {
            let indicator = match p.source {
                PrinterSource::Usb => Span::styled("◆ ", theme::SOURCE_USB),
                _ => match p.status {
                    PrinterStatus::Ready => Span::styled("● ", theme::STATUS_READY),
                    PrinterStatus::Error => Span::styled("✗ ", theme::STATUS_ERROR),
                    PrinterStatus::Offline => Span::styled("✗ ", theme::STATUS_OFFLINE),
                    PrinterStatus::Unknown => {
                        if p.model.is_none() {
                            Span::styled("○ ", theme::DIM)
                        } else {
                            Span::styled("● ", theme::DIM)
                        }
                    }
                },
            };

            let ip_line = Line::from(vec![indicator, Span::raw(p.display_ip())]);
            let model_line = Line::from(Span::styled(
                format!("   {}", p.model.as_deref().unwrap_or("Unknown")),
                theme::DIM,
            ));

            ListItem::new(vec![ip_line, model_line])
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title)
                .border_style(border_style),
        )
        .highlight_style(theme::SELECTED)
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, list_state);
}
