use super::render_activity_panel::activity_widget;
use super::render_help::lines as help_lines;
use super::render_permission::{install_hold_lines, permission_lines};
use super::render_picker::picker_widget;
use super::render_plan::plan_lines;
use super::render_processes::process_lines;
use super::render_selection_picker::widget as selection_picker_widget;
use super::render_sidebar::workspace_widget;
use super::render_subagents::subagent_lines;
use super::render_timeline::timeline_lines;
use super::render_tools::tool_lines;
use super::{render_composer::composer_widget, render_header::header_widget, state::UiState};
use gent_types::{NormalizedTranscriptEvent, NormalizedTranscriptKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
#[path = "render_chips.rs"]
mod render_chips;
#[path = "render_index.rs"]
mod render_index;
#[path = "render_text.rs"]
mod render_text;
pub(super) use render_chips::operational_chips;
use render_index::widget as index_widget;
use render_text::clip;
pub(super) use render_text::selected_title;
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
        frame.render_widget(transcript_widget(state, transcript), transcript);
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
fn transcript_widget(state: &UiState, area: ratatui::layout::Rect) -> Paragraph<'static> {
    if state.help_visible() {
        return Paragraph::new(help_lines(state))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title("Help"));
    }
    if state.activity_visible {
        return activity_widget(state);
    }
    if state.documents_visible || state.templates_visible || state.automations_visible {
        return picker_widget(state);
    }
    if let Some(widget) = super::render_commands::command_widget(state) {
        return widget;
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
    if state.turn_active() {
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
    lines.extend(plan_lines(state));
    if let Some(permission) = state.selected_pending_permission() {
        lines.extend(permission_lines(permission));
    }
    if let Some(hold) = state.selected_install_hold() {
        lines.extend(install_hold_lines(hold));
    }
    let queue = state.selected_queue();
    if !queue.is_empty() {
        lines.push(Line::styled(
            format!(
                "Queued · {} prompt{} · Ctrl+R send now · Ctrl+K remove last",
                queue.len(),
                if queue.len() == 1 { "" } else { "s" }
            ),
            Style::default().fg(Color::Yellow),
        ));
    }
    let inner = area.width.saturating_sub(2);
    let visible = area.height.saturating_sub(2);
    let height = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner);
    let limit = u16::try_from(height)
        .unwrap_or(u16::MAX)
        .saturating_sub(visible);
    let title = match state.scroll() {
        super::state::TranscriptScroll::Pinned(top) if top < limit => {
            "Chat · scrolled back · End follows new messages"
        }
        _ => "Chat",
    };
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((state.transcript_top(limit), 0))
        .block(Block::default().borders(Borders::ALL).title(title))
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

fn transcript_lines(
    events: &[NormalizedTranscriptEvent],
    show_thinking: bool,
) -> Vec<Line<'static>> {
    events
        .iter()
        .flat_map(|event| match event.kind {
            NormalizedTranscriptKind::UserMessage => message_lines("You", event, Color::Yellow),
            NormalizedTranscriptKind::AssistantMessage => message_lines("Gent", event, Color::Cyan),
            NormalizedTranscriptKind::Plan => message_lines("Plan", event, Color::Green),
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
#[cfg(test)]
#[path = "render_stream_tests.rs"]
mod stream_tests;
#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
