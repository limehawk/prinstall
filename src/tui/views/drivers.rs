use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::models::{DiscoveryMethod, DriverResults, DriverSource, MatchConfidence, Printer, PrinterStatus};
use crate::tui::theme;

pub fn render_detail_pane(
    f: &mut Frame,
    area: Rect,
    printer: Option<&Printer>,
    driver_results: Option<&DriverResults>,
    driver_list_state: &mut ListState,
    focused: bool,
    loading_drivers: bool,
) {
    let border_style = if focused {
        theme::FOCUSED_BORDER
    } else {
        theme::UNFOCUSED_BORDER
    };

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Details ")
        .border_style(border_style);

    let Some(p) = printer else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Select a printer to see details",
                theme::DIM,
            )))
            .block(block)
            .alignment(Alignment::Center),
            area,
        );
        return;
    };

    // Split: top info block (7 lines + borders = 9), bottom driver list
    let inner = block.inner(area);
    f.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(" Details ")
            .border_style(border_style),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(3)])
        .split(inner);

    // ── Printer info block ───────────────────────────────────────────────────

    let status_str = match p.status {
        PrinterStatus::Ready => "Ready",
        PrinterStatus::Error => "Error",
        PrinterStatus::Offline => "Offline",
        PrinterStatus::Unknown => "Unknown",
    };

    let ports_str = if p.ports.is_empty() {
        "—".to_string()
    } else {
        p.ports
            .iter()
            .map(|port| port.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let found_str = if p.discovery_methods.is_empty() {
        "—".to_string()
    } else {
        p.discovery_methods
            .iter()
            .map(|m| match m {
                DiscoveryMethod::Snmp => "SNMP",
                DiscoveryMethod::Ipp => "IPP",
                DiscoveryMethod::PortScan => "PortScan",
                DiscoveryMethod::Local => "Local",
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut info_lines = vec![
        Line::from(vec![
            Span::styled(" Model:  ", theme::HEADER),
            Span::raw(p.model.as_deref().unwrap_or("Unknown")),
        ]),
        Line::from(vec![
            Span::styled(" Serial: ", theme::HEADER),
            Span::raw(p.serial.as_deref().unwrap_or("N/A")),
        ]),
        Line::from(vec![
            Span::styled(" Status: ", theme::HEADER),
            Span::raw(status_str),
        ]),
        Line::from(vec![
            Span::styled(" Ports:  ", theme::HEADER),
            Span::raw(ports_str),
        ]),
        Line::from(vec![
            Span::styled(" Found:  ", theme::HEADER),
            Span::raw(found_str),
        ]),
    ];

    if let Some(ref name) = p.local_name {
        info_lines.push(Line::from(vec![
            Span::styled(" Name:   ", theme::HEADER),
            Span::raw(name.clone()),
        ]));
    }

    f.render_widget(Paragraph::new(info_lines), chunks[0]);

    // ── Driver list ───────────────────────────────────────────────────────────

    if loading_drivers {
        f.render_widget(
            Paragraph::new(Span::styled(" Finding drivers...", theme::DIM)),
            chunks[1],
        );
        return;
    }

    let Some(results) = driver_results else {
        f.render_widget(
            Paragraph::new(Span::styled(" Loading drivers...", theme::DIM)),
            chunks[1],
        );
        return;
    };

    let mut items: Vec<ListItem> = Vec::new();
    let mut selectable_indices: Vec<usize> = Vec::new();
    let mut idx: usize = 0;

    if !results.matched.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "── Matched Drivers ──",
            theme::SECTION_HEADER,
        ))));
        idx += 1;

        for dm in &results.matched {
            let badge = match dm.confidence {
                MatchConfidence::Exact => Span::styled("★ exact ", theme::EXACT_BADGE),
                MatchConfidence::Fuzzy => Span::styled("● fuzzy ", theme::FUZZY_BADGE),
                MatchConfidence::Universal => Span::styled("○       ", theme::DIM),
            };
            let source = match dm.source {
                DriverSource::LocalStore => Span::styled(" [Local Store]", theme::DIM),
                DriverSource::Manufacturer => Span::styled(" [Manufacturer]", theme::DIM),
            };
            let line = Line::from(vec![
                Span::raw("  "),
                badge,
                Span::raw(dm.name.clone()),
                source,
            ]);
            selectable_indices.push(idx);
            items.push(ListItem::new(line));
            idx += 1;
        }
    }

    if !results.universal.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "── Universal Drivers ──",
            theme::SECTION_HEADER,
        ))));
        idx += 1;

        for dm in &results.universal {
            let source = match dm.source {
                DriverSource::LocalStore => Span::styled(" [Local Store]", theme::DIM),
                DriverSource::Manufacturer => Span::styled(" [Manufacturer]", theme::DIM),
            };
            let line = Line::from(vec![
                Span::raw("  ○ "),
                Span::raw(dm.name.clone()),
                source,
            ]);
            selectable_indices.push(idx);
            items.push(ListItem::new(line));
            idx += 1;
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Span::styled(" No drivers found", theme::DIM)));
    }

    // driver_list_state tracks a logical index (0..N where N = matched + universal count).
    // selectable_indices maps logical → visual, skipping section header rows.
    let mut visual_state = ListState::default();
    if let Some(logical_idx) = driver_list_state.selected() {
        if let Some(&visual_idx) = selectable_indices.get(logical_idx) {
            visual_state.select(Some(visual_idx));
        }
    }

    let driver_list = List::new(items)
        .highlight_style(theme::SELECTED)
        .highlight_symbol("▶ ");

    f.render_stateful_widget(driver_list, chunks[1], &mut visual_state);
}
