use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{
    commands::{palette, usage},
    state::UiState,
};

pub(super) fn command_widget(state: &UiState) -> Option<Paragraph<'static>> {
    if let Some(cursor) = state.conversation_picker() {
        return Some(conversation_picker(state, cursor));
    }
    let catalog = state.command_catalog()?;
    let matches = palette(catalog, state.input());
    if matches.is_empty() {
        return None;
    }
    let lines = matches
        .iter()
        .map(|command| Line::styled(usage(command), Style::default().fg(Color::Gray)))
        .chain([
            Line::default(),
            Line::styled(
                "Enter runs the typed command · Esc clears",
                Style::default().fg(Color::DarkGray),
            ),
        ])
        .collect::<Vec<_>>();
    Some(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Commands")),
    )
}

fn conversation_picker(state: &UiState, cursor: usize) -> Paragraph<'static> {
    let mut lines = state
        .conversations()
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let title = state
                .metadata(&item.conversation_id)
                .and_then(|metadata| metadata.title.clone())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| item.conversation_id.clone());
            let selected = index == cursor;
            Line::styled(
                format!("{} {title}", if selected { "›" } else { " " }),
                Style::default()
                    .fg(if selected { Color::Cyan } else { Color::Gray })
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )
        })
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.push(Line::styled(
        "↑↓ select · Enter open · Esc close",
        Style::default().fg(Color::Gray),
    ));
    Paragraph::new(lines).wrap(Wrap { trim: true }).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Resume a conversation"),
    )
}
