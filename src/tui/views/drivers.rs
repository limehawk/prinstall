use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::models::*;
use crate::tui::theme;

pub fn render_drivers_view(
    f: &mut Frame,
    area: Rect,
    results: &DriverResults,
    selected: usize,
    loading: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Min(5),   // driver list
            Constraint::Length(2), // help bar
        ])
        .split(area);

    // Title
    let title_text = if loading {
        "Finding drivers...".to_string()
    } else {
        format!("Drivers for: {}", results.printer_model)
    };
    let title = Paragraph::new(title_text).style(theme::TITLE);
    let title_block = Block::default().borders(Borders::BOTTOM);
    f.render_widget(title.block(title_block), chunks[0]);

    // Build combined list with section headers
    let mut items: Vec<ListItem> = Vec::new();
    let mut selectable_indices: Vec<usize> = Vec::new();
    let mut idx = 0;

    if !results.matched.is_empty() {
        items.push(ListItem::new(Line::from(
            Span::styled("── Matched Drivers ──", theme::SECTION_HEADER),
        )));
        idx += 1;

        for (i, dm) in results.matched.iter().enumerate() {
            let badge = match dm.confidence {
                MatchConfidence::Exact => Span::styled(" ★ exact ", theme::EXACT_BADGE),
                MatchConfidence::Fuzzy => Span::styled(" ● fuzzy ", theme::FUZZY_BADGE),
                MatchConfidence::Universal => Span::styled(" ○ ", theme::DIM),
            };
            let source = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            let num = i + 1;
            let line = Line::from(vec![
                Span::raw(format!("  #{num:<2} ")),
                Span::raw(&dm.name),
                badge,
                Span::styled(format!(" {source}"), theme::DIM),
            ]);
            selectable_indices.push(idx);
            items.push(ListItem::new(line));
            idx += 1;
        }
    }

    if !results.universal.is_empty() {
        items.push(ListItem::new(Line::from(
            Span::styled("── Universal Drivers ──", theme::SECTION_HEADER),
        )));
        idx += 1;

        let offset = results.matched.len();
        for (i, dm) in results.universal.iter().enumerate() {
            let source = match dm.source {
                DriverSource::LocalStore => "[Local Store]",
                DriverSource::Manufacturer => "[Manufacturer]",
            };
            let num = offset + i + 1;
            let line = Line::from(vec![
                Span::raw(format!("  #{num:<2} ")),
                Span::raw(&dm.name),
                Span::styled(format!("  {source}"), theme::DIM),
            ]);
            selectable_indices.push(idx);
            items.push(ListItem::new(line));
            idx += 1;
        }
    }

    // Highlight selected item
    if let Some(&visual_idx) = selectable_indices.get(selected)
        && let Some(item) = items.get_mut(visual_idx)
    {
        *item = item.clone().style(theme::SELECTED);
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Drivers "));
    f.render_widget(list, chunks[1]);

    // Help bar
    let help = Line::from(vec![
        Span::styled("↑↓", theme::HELP_KEY),
        Span::styled(" navigate  ", theme::HELP_TEXT),
        Span::styled("Enter", theme::HELP_KEY),
        Span::styled(" install  ", theme::HELP_TEXT),
        Span::styled("Esc", theme::HELP_KEY),
        Span::styled(" back  ", theme::HELP_TEXT),
        Span::styled("q", theme::HELP_KEY),
        Span::styled(" quit", theme::HELP_TEXT),
    ]);
    f.render_widget(Paragraph::new(help), chunks[2]);
}
