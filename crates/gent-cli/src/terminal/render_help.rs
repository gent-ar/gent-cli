use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

use super::{
    commands::{usage, visible},
    state::UiState,
};

pub(super) fn lines(state: &UiState) -> Vec<Line<'static>> {
    let shortcuts: Vec<Line<'static>> = [
        (
            "Start",
            "Ctrl+N creates a chat in the current workspace; a focused session receives that new chat.",
        ),
        (
            "Navigate",
            "↑/↓ chooses a chat. PgUp/PgDn reads history.",
        ),
        (
            "Send",
            "Type a message and press Enter. Shift+Enter or Alt+Enter adds a line. Type / to list commands.",
        ),
        (
            "Selection",
            "Tab provider · Ctrl+L model · Ctrl+E effort · Ctrl+O mode. Enter applies a context-preserving switch; Ctrl+P changes permissions; Ctrl+G opens sessions.",
        ),
        (
            "Context",
            "Ctrl+X toggles preserved or cleared context for the next model, effort, mode, or provider switch.",
        ),
        (
            "Files",
            "Drag or paste a file path to attach it.",
        ),
        (
            "Activity",
            "F2 opens the live tools, processes, subagents, permissions, and timeline.",
        ),
        (
            "Thinking",
            "Ctrl+T shows or summarizes provider-emitted thinking and saves that preference. Ctrl+C interrupts the active prompt.",
        ),
        ("Close", "F1, ?, or Esc returns to the chat. Ctrl+Q quits."),
    ]
    .into_iter()
    .flat_map(|(label, detail)| section(label, [format!("  {detail}")]))
    .collect();
    let commands = state.command_catalog().map_or_else(
        || vec!["  Gentd has not listed commands yet.".into()],
        |catalog| {
            catalog
                .commands
                .iter()
                .filter(|command| visible(command))
                .map(|command| format!("  {}", usage(command)))
                .collect::<Vec<_>>()
        },
    );
    let mut lines = section("Commands", commands);
    lines.extend(shortcuts);
    lines
}

fn section(label: &'static str, details: impl IntoIterator<Item = String>) -> Vec<Line<'static>> {
    std::iter::once(Line::styled(
        label,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
    .chain(details.into_iter().map(Line::from))
    .chain(std::iter::once(Line::default()))
    .collect()
}
