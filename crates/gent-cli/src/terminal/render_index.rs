use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem},
};

use super::{
    super::state::UiState,
    render_text::{clip, plural, title_for},
};

pub(super) fn widget(state: &UiState) -> List<'static> {
    let items = state
        .visible_conversation_indices()
        .into_iter()
        .map(|index| {
            let item = &state.conversations()[index];
            let title = title_for(state, &item.conversation_id)
                .unwrap_or_else(|| format!("Conversation {}", index + 1));
            let mut lines = vec![
                Line::styled(title, Style::default().add_modifier(Modifier::BOLD)),
                Line::styled(
                    format!("{} run{}", item.run_count, plural(item.run_count)),
                    Style::default().fg(Color::DarkGray),
                ),
            ];
            if let Some(preview) = state
                .metadata(&item.conversation_id)
                .and_then(|value| value.preview.as_deref())
            {
                lines.push(Line::styled(
                    clip(preview, 72),
                    Style::default().fg(Color::Gray),
                ));
            } else if let Some(recap) = state
                .metadata(&item.conversation_id)
                .and_then(|value| value.recap.as_deref())
            {
                lines.push(Line::styled(
                    clip(recap, 72),
                    Style::default().fg(Color::Gray),
                ));
            }
            let sessions = state
                .sessions()
                .iter()
                .filter(|session| session.conversation_ids.contains(&item.conversation_id))
                .map(|session| session.name.as_str())
                .collect::<Vec<_>>();
            if !sessions.is_empty() {
                lines.push(Line::styled(
                    format!("Session: {}", sessions.join(", ")),
                    Style::default().fg(Color::Cyan),
                ));
            }
            ListItem::new(lines)
        })
        .collect::<Vec<_>>();
    List::new(if items.is_empty() {
        vec![ListItem::new(
            "No conversations match. /search clear resets.",
        )]
    } else {
        items
    })
    .highlight_style(Style::default().fg(Color::Cyan))
    .highlight_symbol("› ")
    .block(Block::default().borders(Borders::ALL).title(
        state.conversation_filter().map_or_else(
            || {
                format!(
                    "Conversations  Sessions: {} (Ctrl+G)",
                    state.sessions().len()
                )
            },
            |filter| format!("Conversations · {filter} · /search clear"),
        ),
    ))
}
