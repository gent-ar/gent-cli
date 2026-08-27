use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::{
    render::{operational_chips, selected_title},
    state::UiState,
};

pub(super) fn header_widget(state: &UiState, width: u16) -> Paragraph<'static> {
    let mut headline = vec![
        Span::styled("gent", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  /  "),
        Span::styled(selected_title(state), Style::default().fg(Color::Cyan)),
    ];
    if let Some(workspace) = state.selected_workspace_path() {
        headline.extend([
            Span::raw("  ·  "),
            Span::styled(workspace_name(workspace), Style::default().fg(Color::Gray)),
        ]);
    }
    Paragraph::new(vec![
        Line::from(headline),
        Line::styled(
            "↑↓ chats · Ctrl+G sessions · Ctrl+N new · F1 help · F2 activity · Ctrl+Q quit",
            Style::default().fg(Color::DarkGray),
        ),
        Line::from(operational_chips(state, width)),
    ])
    .block(Block::default().borders(Borders::ALL))
}

fn workspace_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .chars()
        .take(40)
        .collect()
}
