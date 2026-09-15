use super::super::render_activity::counts as activity_counts;
use super::super::state::UiState;
use super::render_text::status_activity;
use gent_types::ConversationActivityFact;
use ratatui::{
    style::{Color, Style},
    text::Span,
};

pub(in crate::terminal) fn operational_chips(state: &UiState, width: u16) -> Vec<Span<'static>> {
    let (tools, subagents, processes) = activity_counts(state.selected_activity());
    let activity = if state.awaiting_turn() {
        "preparing"
    } else {
        state.selected_status().map_or("idle", status_activity)
    };
    let activity_count = state.selected_activity().len();
    let mut values = vec![
        format!("[ {activity} ]"),
        context_label(state.selected_activity()).unwrap_or_else(|| "[ context 0% ]".into()),
    ];
    if state.selection().mode == gent_types::AgentChatMode::Plan {
        values.push("[ planning ]".into());
    }
    if let Some(files) = state.selected_changed_file_count() {
        values.push(format!("[ {files} files ]"));
    }
    if state.selected_pending_permission().is_some() {
        values.push("[ permission ]".into());
    }
    if state.selected_status().is_some_and(|status| {
        status.runs.iter().any(|run| {
            run.live_status
                .as_ref()
                .is_some_and(|live| live.status.has_error())
        })
    }) {
        values.push("[ error ]".into());
    }
    if state.selected_status().is_some_and(|status| {
        status.runs.iter().any(|run| {
            run.live_status
                .as_ref()
                .is_some_and(|live| live.status.needs_attention())
        })
    }) {
        values.push("[ attention ]".into());
    }
    values.push(format!("[ {activity_count} activity ]"));
    if tools > 0 {
        values.push(format!("[ {tools} tools ]"));
    }
    if processes > 0 {
        values.push(format!("[ {processes} processes ]"));
    }
    if subagents > 0 {
        values.push(format!("[ {subagents} subagents ]"));
    }
    let mcp_servers = state.selected_mcp_server_count();
    if mcp_servers > 0 {
        values.push(format!("[ MCP {mcp_servers} ]"));
    }
    let automations = state.selected_automation_count();
    if automations > 0 {
        values.push(format!("[ {automations} automations ]"));
    }
    let forge = state.selected_forge_count();
    if forge > 0 {
        values.push(format!("[ Forge {forge} ]"));
    }
    if let Some(branch) = state.selected_git_branch() {
        values.push(format!("[ {branch} ]"));
    }
    let runs = state
        .selected_status()
        .map_or(0, |status| status.runs.len());
    values.push(format!("[ {runs} runs ]"));
    let available = usize::from(width.saturating_sub(2));
    let mut used = 0;
    values
        .into_iter()
        .take_while(|value| {
            let next = used + value.len() + usize::from(used > 0);
            if next > available {
                return false;
            }
            used = next;
            true
        })
        .flat_map(|value| {
            [
                Span::styled(value, Style::default().fg(Color::Green)),
                Span::raw(" "),
            ]
        })
        .collect()
}
fn context_label(facts: &[ConversationActivityFact]) -> Option<String> {
    let (used_tokens, window_tokens) = facts.iter().rev().find_map(|fact| match fact {
        ConversationActivityFact::ContextUsage {
            used_tokens,
            window_tokens,
            ..
        } => Some((*used_tokens, *window_tokens)),
        ConversationActivityFact::TokenUsage { usage, .. } => Some((usage.prompt_tokens(), None)),
        _ => None,
    })?;
    Some(window_tokens.filter(|value| *value > 0).map_or_else(
        || format!("[ context {} ]", compact_number(used_tokens)),
        |window| format!("[ context {}% ]", used_tokens.saturating_mul(100) / window),
    ))
}
fn compact_number(value: u64) -> String {
    if value >= 1_000 {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}
