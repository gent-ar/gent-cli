use super::render_activity::counts as activity_counts;
use super::render_activity_panel::activity_widget;
use super::render_help::lines as help_lines;
use super::render_permission::permission_lines;
use super::render_picker::picker_widget;
use super::render_processes::process_lines;
use super::render_selection_picker::widget as selection_picker_widget;
use super::render_sidebar::workspace_widget;
use super::render_subagents::subagent_lines;
use super::render_timeline::timeline_lines;
use super::render_tools::tool_lines;
use super::{render_composer::composer_widget, render_header::header_widget, state::UiState};
use gent_types::{ConversationActivityFact, NormalizedTranscriptEvent, NormalizedTranscriptKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
#[path = "render_index.rs"]
mod render_index;
#[path = "render_text.rs"]
mod render_text;
use render_index::widget as index_widget;
pub(super) use render_text::selected_title;
use render_text::{clip, status_activity};
pub(crate) fn render(frame: &mut Frame, state: &UiState) {
    let [header, body] =
        Layout::vertical([Constraint::Length(5), Constraint::Min(8)]).areas(frame.area());
    let [sidebar, main] =
        Layout::horizontal([Constraint::Percentage(28), Constraint::Percentage(72)]).areas(body);
    let [transcript, composer] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(8)]).areas(main);
    let session_height = if state.sessions().is_empty() { 0 } else { 4 };
    let workspace_height = sidebar.height.saturating_sub(session_height + 5).min(18);
    let [sessions, index, workspace] = Layout::vertical([
        Constraint::Length(session_height),
        Constraint::Min(5),
        Constraint::Length(workspace_height),
    ])
    .areas(sidebar);
    frame.render_widget(header_widget(state, header.width), header);
    let mut session_state = ListState::default();
    session_state.select(state.selected_session_index());
    frame.render_stateful_widget(session_widget(state), sessions, &mut session_state);
    let mut list_state = ListState::default();
    list_state.select(
        state
            .visible_conversation_indices()
            .iter()
            .position(|index| Some(*index) == state.selected_index()),
    );
    frame.render_stateful_widget(index_widget(state), index, &mut list_state);
    frame.render_widget(
        workspace_widget(state, workspace.width, workspace.height),
        workspace,
    );
    if let Some((picker, mut picker_state)) = selection_picker_widget(state) {
        frame.render_stateful_widget(picker, transcript, &mut picker_state);
    } else {
        frame.render_widget(transcript_widget(state, transcript.height), transcript);
    }
    frame.render_widget(composer_widget(state), composer);
}
fn session_widget(state: &UiState) -> List<'static> {
    let items: Vec<_> = state
        .sessions()
        .iter()
        .map(|session| ListItem::new(session.name.clone()))
        .collect();
    List::new(items)
        .highlight_symbol("› ")
        .highlight_style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL).title("Sessions"))
}
fn transcript_widget(state: &UiState, height: u16) -> Paragraph<'static> {
    if state.help_visible() {
        return Paragraph::new(help_lines())
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Help"));
    }
    if state.activity_visible {
        return activity_widget(state);
    }
    if state.documents_visible || state.templates_visible || state.automations_visible {
        return picker_widget(state);
    }
    let events = state.selected_transcript();
    let mut lines = if events.is_empty() {
        vec![Line::styled(
            "No messages yet. Write below to begin.",
            Style::default().fg(Color::DarkGray),
        )]
    } else {
        transcript_lines(events, state.show_thinking())
    };
    if state.awaiting_turn()
        || state
            .selected_status()
            .is_some_and(|status| status.runs.iter().any(|run| run.active_turn_id.is_some()))
    {
        lines.push(Line::styled(
            active_turn_label(state),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    let timeline = timeline_lines(state);
    if !timeline.is_empty() {
        lines.splice(0..0, timeline);
    }
    if let Some(permission) = state.selected_pending_permission() {
        lines.splice(0..0, permission_lines(permission));
    }
    let tools = tool_lines(state.selected_activity());
    if !tools.is_empty() {
        lines.splice(0..0, tools);
    }
    let subagents = subagent_lines(state.selected_activity());
    if !subagents.is_empty() {
        lines.splice(0..0, subagents);
    }
    let processes = process_lines(state.selected_activity());
    if !processes.is_empty() {
        lines.splice(0..0, processes);
    }
    let scroll = scroll_for_latest(&lines, height, state.scroll_offset());
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(Block::default().borders(Borders::ALL).title("Chat"))
}

fn active_turn_label(state: &UiState) -> String {
    match state.selected_local_model_state() {
        Some(gent_protocol::LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        }) if *total_bytes > 0 => format!(
            "Downloading {} model - {}% - Cancel [Ctrl+C]",
            state.selection().model,
            downloaded_bytes.saturating_mul(100) / total_bytes,
        ),
        Some(gent_protocol::LocalModelInstallState::NotInstalled) => {
            format!("Preparing {} model download…", state.selection().model,)
        }
        _ => {
            let mut live = state.selected_status().into_iter().flat_map(|status| {
                status
                    .runs
                    .iter()
                    .filter_map(|run| run.live_status.as_ref())
                    .map(|live| &live.status)
            });
            if live
                .clone()
                .any(gent_types::ConversationLiveStatus::is_waiting_for_subagents)
            {
                "Waiting for subagents…".into()
            } else if live
                .clone()
                .any(gent_types::ConversationLiveStatus::is_waiting_for_command)
            {
                "Waiting for command…".into()
            } else if live
                .clone()
                .any(gent_types::ConversationLiveStatus::needs_attention)
            {
                "Waiting for your response…".into()
            } else if live.any(gent_types::ConversationLiveStatus::has_error) {
                "Provider reported an error.".into()
            } else {
                "Gent is thinking…".into()
            }
        }
    }
}

fn scroll_for_latest(lines: &[Line<'_>], height: u16, older_offset: u16) -> u16 {
    let visible = usize::from(height.saturating_sub(2));
    let end = lines.len().saturating_sub(visible);
    end.saturating_sub(usize::from(older_offset))
        .try_into()
        .unwrap_or(u16::MAX)
}
fn transcript_lines(
    events: &[NormalizedTranscriptEvent],
    show_thinking: bool,
) -> Vec<Line<'static>> {
    events
        .iter()
        .flat_map(|event| match event.kind {
            NormalizedTranscriptKind::UserMessage => message_lines("You", event, Color::Yellow),
            NormalizedTranscriptKind::AssistantMessage => message_lines("Gent", event, Color::Cyan),
            NormalizedTranscriptKind::Thinking if show_thinking => {
                message_lines("Thinking", event, Color::DarkGray)
            }
            NormalizedTranscriptKind::Thinking => {
                vec![Line::styled(
                    "  · thinking",
                    Style::default().fg(Color::DarkGray),
                )]
            }
            NormalizedTranscriptKind::ToolActivity => message_lines("Tool", event, Color::Magenta),
            NormalizedTranscriptKind::Notice => message_lines("Notice", event, Color::DarkGray),
        })
        .collect()
}
fn message_lines(
    label: &'static str,
    event: &NormalizedTranscriptEvent,
    color: Color,
) -> Vec<Line<'static>> {
    let text = clip(&event.text, 1_800);
    let mut lines = vec![Line::styled(
        format!(
            "{label}{}",
            if event.is_partial {
                " · streaming"
            } else {
                ""
            }
        ),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )];
    lines.extend(text.lines().map(|line| Line::from(format!("  {line}"))));
    lines.push(Line::default());
    lines
}
pub(super) fn operational_chips(state: &UiState, width: u16) -> Vec<Span<'static>> {
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
#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
