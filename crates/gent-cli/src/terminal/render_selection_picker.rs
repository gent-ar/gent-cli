use ratatui::{
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use super::state::UiState;

pub(super) fn widget(state: &UiState) -> Option<(List<'static>, ListState)> {
    let (title, values, selected) = state.picker_view()?;
    let items = values.into_iter().map(ListItem::new).collect::<Vec<_>>();
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    Some((
        List::new(items)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(Color::Cyan))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{title} · Enter apply · Esc cancel")),
            ),
        list_state,
    ))
}
