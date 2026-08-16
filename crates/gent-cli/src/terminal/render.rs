//! Pure Ratatui rendering for conversation discovery and durable-input availability.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use super::state::UiState;

pub(crate) fn render(frame: &mut Frame, state: &UiState) {
    let [header, body, composer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(5),
    ])
    .areas(frame.area());
    frame.render_widget(header_widget(), header);
    let [index, detail] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(body);
    frame.render_widget(index_widget(state), index);
    frame.render_widget(detail_widget(state), detail);
    frame.render_widget(composer_widget(state), composer);
}

fn header_widget() -> Paragraph<'static> {
    Paragraph::new("Gent conversations  •  ↑/k ↓/j select  •  q quit")
        .block(Block::default().borders(Borders::ALL).title("gent"))
}

fn index_widget(state: &UiState) -> List<'static> {
    let items = if state.conversations().is_empty() {
        vec![ListItem::new("No durable conversations yet.")]
    } else {
        state
            .conversations()
            .iter()
            .map(|item| {
                let marker = if state.selected() == Some(item) {
                    "›"
                } else {
                    " "
                };
                ListItem::new(format!(
                    "{marker} {}  ({} runs)",
                    item.conversation_id, item.run_count
                ))
            })
            .collect()
    };
    List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Conversations"),
    )
}

fn detail_widget(state: &UiState) -> Paragraph<'static> {
    let body = state.selected().map_or_else(
        || "Select a conversation when one is available.\n\nNo transcript content is exposed in observer mode.".into(),
        |item| format!("Conversation: {}\nRuns: {}\n\nUse gent conversation status or timeline for content-free metadata.", item.conversation_id, item.run_count),
    );
    Paragraph::new(body)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("Details"))
}

fn composer_widget(state: &UiState) -> Paragraph<'static> {
    let body = if state.chat_enabled() {
        format!(
            "{}\nProvider: {:?} (Tab)  Effort: {:?} (Ctrl+E)  Mode: {:?} (Ctrl+M)\nCtrl+N create  •  Enter persist prompt  •  {}",
            state.input(),
            state.selection().provider,
            state.selection().effort,
            state.selection().mode,
            state
                .notice()
                .unwrap_or("No provider lifecycle is connected."),
        )
    } else {
        "Chat execution is unavailable while gentd is in observer mode.\nPrompt input and model, effort, and mode switches are disabled.".into()
    };
    Paragraph::new(body)
        .style(Style::default().fg(if state.chat_enabled() {
            Color::Green
        } else {
            Color::Yellow
        }))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if state.chat_enabled() {
                    "Input"
                } else {
                    "Input (disabled)"
                }),
        )
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::terminal::UiState;

    #[test]
    fn observer_render_states_the_disabled_boundary_without_transcript_content() {
        let backend = TestBackend::new(90, 22);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &UiState::new(Vec::new())))
            .unwrap();
        let output = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        assert!(output.contains("Input (disabled)"));
        assert!(output.contains("Chat execution is unavailable"));
        assert!(!output.contains("private prompt"));
    }
}
