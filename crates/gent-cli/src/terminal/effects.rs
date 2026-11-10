use std::{
    io,
    time::{Duration, Instant},
};

use super::{
    state::{UiCommand, UiEffect, UiRequest, UiState},
    terminal_loop::TerminalPorts,
};

const LIVE_REFRESH: Duration = Duration::from_millis(250);
const SETTLE: Duration = Duration::from_secs(3);

pub(super) fn refresh(state: &mut UiState, ports: &mut TerminalPorts<'_>, conversation_id: String) {
    let was_live = state.turn_active();
    match (ports.refresh)(conversation_id) {
        Ok(view) => state.apply_view(view),
        Err(error) => return state.set_notice(error),
    }
    refresh_workspace_git(state, ports, was_live && !state.turn_active());
}

pub(super) fn refresh_selected(state: &mut UiState, ports: &mut TerminalPorts<'_>) {
    if let Some(conversation_id) = state.selected().map(|item| item.conversation_id.clone()) {
        refresh(state, ports, conversation_id);
    }
}

pub(super) fn refresh_workspace_git(
    state: &mut UiState,
    ports: &mut TerminalPorts<'_>,
    turn_ended: bool,
) {
    if let Some(workspace_id) = state.workspace_git_due(turn_ended) {
        (ports.workspace_git)(Some(workspace_id));
    }
}

fn poll_workspace_git(state: &mut UiState, ports: &mut TerminalPorts<'_>) {
    if let Some((workspace_id, result)) = (ports.workspace_git)(None) {
        match result {
            Ok(report) => state.set_workspace_git(workspace_id, report),
            Err(error) => {
                state.set_workspace_git(workspace_id, None);
                state.set_notice(error);
            }
        }
    }
}

pub(super) struct LiveClock {
    last: Instant,
    idle_since: Option<Instant>,
}

impl LiveClock {
    pub(super) fn new() -> Self {
        Self {
            last: Instant::now(),
            idle_since: None,
        }
    }
}

pub(super) fn live_tick(state: &mut UiState, ports: &mut TerminalPorts<'_>, clock: &mut LiveClock) {
    poll_workspace_git(state, ports);
    if clock.last.elapsed() < LIVE_REFRESH {
        return;
    }
    clock.last = Instant::now();
    let settling = if state.turn_active() {
        clock.idle_since = None;
        true
    } else {
        clock.idle_since.get_or_insert_with(Instant::now).elapsed() < SETTLE
    };
    if settling {
        refresh_selected(state, ports);
    }
}

pub(super) fn apply_command(
    state: &mut UiState,
    ports: &mut TerminalPorts<'_>,
    command: UiCommand,
    suspend: &mut dyn FnMut(bool) -> io::Result<()>,
) -> io::Result<bool> {
    let previous_thinking = state.show_thinking();
    match state.apply(command) {
        UiEffect::Quit => return Ok(true),
        UiEffect::Request(value) => {
            let clears_composer = matches!(
                &value,
                UiRequest::Send { .. } | UiRequest::Queue { .. } | UiRequest::InvokeCommand { .. }
            );
            match (ports.request)(value) {
                Ok(result) => {
                    if clears_composer {
                        state.clear_sent_prompt();
                    }
                    state.apply_request(result);
                    refresh_selected(state, ports);
                }
                Err(error) => state.set_notice(error),
            }
        }
        UiEffect::RenderTemplate {
            template_id,
            variables,
        } => match (ports.render_template)(template_id, variables) {
            Ok(prompt) => {
                state.replace_input(prompt);
                state.set_notice("Template rendered. Press Enter to send.".into());
            }
            Err(error) => state.set_notice(error),
        },
        UiEffect::Refresh(conversation_id) => refresh(state, ports, conversation_id),
        UiEffect::LoadConversations => match (ports.conversations)() {
            Ok((conversations, metadata)) => state.merge_conversations(conversations, metadata),
            Err(error) => state.set_notice(error),
        },
        UiEffect::ListDocuments {
            workspace_id,
            attach_id,
        } => match (ports.list_documents)(workspace_id) {
            Ok(documents) => state.set_documents(documents, attach_id),
            Err(error) => state.set_notice(error),
        },
        UiEffect::ListTemplates => match (ports.list_templates)() {
            Ok(templates) => state.set_templates(templates),
            Err(error) => state.set_notice(error),
        },
        UiEffect::CreateSession(session) => match (ports.create_session)(session) {
            Ok(session) => state.add_session(session),
            Err(error) => state.set_notice(error),
        },
        UiEffect::Login(provider) => {
            suspend(true)?;
            let result = (ports.login)(provider);
            suspend(false)?;
            match result {
                Ok(notice) | Err(notice) => state.set_notice(notice),
            }
        }
        UiEffect::Continue => {}
    }
    if previous_thinking != state.show_thinking()
        && let Err(error) = (ports.save_thinking)(state.show_thinking())
    {
        state.set_notice(error);
    }
    Ok(false)
}
