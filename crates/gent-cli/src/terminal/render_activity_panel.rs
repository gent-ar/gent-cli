use ratatui::{
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{
    render_activity::timeline_lines as activity_timeline_lines,
    render_permission::permission_lines, render_timeline::timeline_lines, state::UiState,
};

pub(super) fn activity_widget(state: &UiState) -> Paragraph<'static> {
    let mut lines = timeline_lines(state);
    if let Some(permission) = state.selected_pending_permission() {
        lines.extend(permission_lines(permission));
        lines.push(Line::default());
    }
    lines.extend(activity_timeline_lines(state.selected_activity()));
    if lines.is_empty() {
        lines.push(Line::styled(
            "No activity recorded for this conversation yet.",
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "Esc returns to chat.",
        Style::default().fg(Color::Gray),
    ));
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title("Activity"))
}
