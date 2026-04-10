use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::tui::theme;

pub fn render_help_overlay(f: &mut Frame, area: Rect) {
    let popup_width = 52u16.min(area.width);
    let popup_height = 20u16.min(area.height);
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_rect = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_rect);

    let help_lines = vec![
        Line::from(Span::styled("Navigation", theme::HEADER)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  j/k         ", theme::HELP_KEY),
            Span::styled("Move up/down in lists", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  h/l         ", theme::HELP_KEY),
            Span::styled("Move focus between panels", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  Tab         ", theme::HELP_KEY),
            Span::styled("Cycle panel focus", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  g/G         ", theme::HELP_KEY),
            Span::styled("Jump to top/bottom", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  Enter       ", theme::HELP_KEY),
            Span::styled("Select / install driver", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  Esc         ", theme::HELP_KEY),
            Span::styled("Back / close overlay", theme::HELP_TEXT),
        ]),
        Line::from(""),
        Line::from(Span::styled("Actions", theme::HEADER)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  s           ", theme::HELP_KEY),
            Span::styled("Rescan", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  ?           ", theme::HELP_KEY),
            Span::styled("Toggle this help", theme::HELP_TEXT),
        ]),
        Line::from(vec![
            Span::styled("  q           ", theme::HELP_KEY),
            Span::styled("Quit", theme::HELP_TEXT),
        ]),
    ];

    f.render_widget(
        Paragraph::new(help_lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Help — press ? or Esc to close ")
                .border_style(theme::FOCUSED_BORDER),
        ),
        popup_rect,
    );
}
