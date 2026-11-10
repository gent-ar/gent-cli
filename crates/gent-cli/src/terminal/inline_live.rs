use std::time::Duration;

use gent_types::{ConversationActivityFact, GoalStatus, NormalizedTranscriptKind};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use super::{
    render::active_turn_label, render_permission::install_hold_lines,
    render_subagents::subagent_lines, state::UiState, state::state_inline::PERMISSION_CHOICES,
};

pub(crate) fn live_lines(state: &UiState, elapsed: Option<Duration>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(goal) = goal_line(state.selected_activity()) {
        lines.push(goal);
    }
    if state.turn_active() {
        lines.extend(subagent_lines(state.selected_activity()));
    }
    if let Some(request) = state.selected_pending_permission() {
        lines.extend(permission_menu(request, state.inline().permission_cursor));
    }
    if let Some(hold) = state.selected_install_hold() {
        lines.extend(install_hold_lines(hold));
    }
    lines.extend(queue_lines(state));
    if state.turn_active() {
        let seconds = elapsed.map_or(String::new(), |value| format!("{}s · ", value.as_secs()));
        lines.push(Line::from(vec![
            Span::styled("✻ ", Style::default().fg(Color::Cyan)),
            Span::styled(
                match active_turn_label(state) {
                    _ if state.selected_pending_permission().is_some() => {
                        "Waiting for your answer".into()
                    }
                    label
                        if label.starts_with("Waiting for your")
                            || label.starts_with("Gent is") =>
                    {
                        "Working…".into()
                    }
                    label => label,
                },
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({seconds}ctrl+c to interrupt)"),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    lines
}

fn goal_line(facts: &[ConversationActivityFact]) -> Option<Line<'static>> {
    let goal = facts.iter().rev().find_map(|fact| match fact {
        ConversationActivityFact::GoalUpdated { goal, .. } => Some(goal),
        _ => None,
    })?;
    if matches!(goal.status, GoalStatus::Complete | GoalStatus::Cleared) {
        return None;
    }
    Some(Line::from(vec![
        Span::styled("◎ Goal  ", Style::default().fg(Color::Magenta)),
        Span::raw(goal.objective.lines().next().unwrap_or_default().to_owned()),
        Span::styled(
            format!(" · {:?}", goal.status).to_lowercase(),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

fn queue_lines(state: &UiState) -> Vec<Line<'static>> {
    let queue = state.selected_queue();
    queue
        .iter()
        .enumerate()
        .map(|(index, prompt)| {
            let event_id = format!("user:{}", prompt.message_id);
            let text = state
                .selected_transcript()
                .iter()
                .find(|event| {
                    event.event_id == event_id
                        && event.kind == NormalizedTranscriptKind::UserMessage
                })
                .map_or("queued prompt", |event| {
                    event.text.lines().next().unwrap_or_default()
                })
                .to_owned();
            let mut spans = vec![
                Span::styled("◷ Queued  ", Style::default().fg(Color::Yellow)),
                Span::raw(text),
            ];
            if index == 0 {
                spans.push(Span::styled(
                    "   ctrl+r send now · ctrl+k remove last",
                    Style::default().fg(Color::DarkGray),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

pub(crate) fn permission_menu(
    request: &gent_types::PermissionDecisionRequest,
    cursor: usize,
) -> Vec<Line<'static>> {
    let input = request.request.input.as_ref();
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| {
                input
                    .and_then(|value| value.get(*key))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned)
    };
    let category = format!("{:?}", request.request.category).to_lowercase();
    let proposed = super::inline_tools::proposed_change(&request.request.tool_name, input);
    let (question, detail) = if let Some(command) = field(&["command", "cmd"]) {
        (
            "Run this command?".to_owned(),
            Some(super::inline_tools::command_text(&command)),
        )
    } else if let Some(path) = proposed.0.clone() {
        (
            format!("Allow {} on this file?", request.request.tool_name),
            Some(path),
        )
    } else {
        (
            format!("Allow {}?", request.request.tool_name),
            field(&["description", "url", "query", "pattern", "prompt"]),
        )
    };
    let yellow = Style::default().fg(Color::Yellow);
    let mut lines = vec![Line::from(vec![
        Span::styled("◆ ", yellow),
        Span::styled(question, yellow.add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("  {} · {category}", request.request.tool_name),
            Style::default().fg(Color::DarkGray),
        ),
    ])];
    if let Some(detail) = detail {
        lines.extend(detail.lines().take(4).map(|line| {
            Line::styled(
                format!("    {line}"),
                Style::default().add_modifier(Modifier::BOLD),
            )
        }));
    }
    lines.extend(proposed.1);
    for (index, (_, label)) in PERMISSION_CHOICES.iter().enumerate() {
        let label = match index {
            1 => format!("{label} ({})", request.request.tool_name),
            2 => format!("{label} ({category})"),
            _ => (*label).to_owned(),
        };
        let selected = index == cursor;
        lines.push(Line::styled(
            format!(
                "  {} {}. {label}",
                if selected { "❯" } else { " " },
                index + 1
            ),
            if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ));
    }
    lines.push(Line::styled(
        "  press 1-4, or ↑↓ and Enter",
        Style::default().fg(Color::DarkGray),
    ));
    lines
}
