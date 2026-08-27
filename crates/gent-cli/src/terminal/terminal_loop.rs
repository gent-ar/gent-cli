use super::{
    input, render,
    state::{UiCommand, UiEffect, UiRequest, UiRequestResult, UiState},
};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gent_types::PromptTemplateVariable;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal, Stdout},
    time::{Duration, Instant},
};
pub(crate) fn run<F, R, T>(
    mut state: UiState,
    mut request: F,
    mut refresh: R,
    mut render_template: T,
    mut list_documents: impl FnMut(
        String,
    ) -> Result<Vec<gent_protocol::WorkspaceDocumentRecord>, String>,
    mut list_templates: impl FnMut() -> Result<Vec<gent_types::PromptTemplateRecord>, String>,
    mut create_session: impl FnMut(
        gent_types::AgentChatSession,
    ) -> Result<gent_types::AgentChatSession, String>,
    mut login: impl FnMut(gent_types::AgentChatProvider) -> Result<String, String>,
    mut save_thinking: impl FnMut(bool) -> Result<(), String>,
) -> io::Result<()>
where
    F: FnMut(UiRequest) -> Result<UiRequestResult, String>,
    R: FnMut(String) -> Result<super::ConversationView, String>,
    T: FnMut(String, Vec<PromptTemplateVariable>) -> Result<String, String>,
{
    require_interactive()?;
    let mut terminal = TerminalSession::open()?;
    let mut last_live_refresh = Instant::now();
    loop {
        terminal
            .terminal
            .draw(|frame| render::render(frame, &state))?;
        if event::poll(Duration::from_millis(100))? {
            let command = match event::read()? {
                Event::Key(key) => input::command(key, state.chat_enabled()),
                Event::Paste(value) if state.chat_enabled() => Some(UiCommand::Paste(value)),
                _ => None,
            };
            if let Some(command) = command {
                let previous_thinking = state.show_thinking();
                match state.apply(command) {
                    UiEffect::Quit => return Ok(()),
                    UiEffect::Request(value) => {
                        let clears_composer = matches!(&value, UiRequest::Send { .. });
                        match request(value) {
                            Ok(result) => {
                                if clears_composer {
                                    state.clear_sent_prompt();
                                }
                                state.apply_request(result);
                                if let Some(conversation_id) =
                                    state.selected().map(|item| item.conversation_id.clone())
                                {
                                    match refresh(conversation_id) {
                                        Ok(view) => state.apply_view(view),
                                        Err(error) => state.set_notice(error),
                                    }
                                }
                            }
                            Err(error) => state.set_notice(error),
                        }
                    }
                    UiEffect::RenderTemplate {
                        template_id,
                        variables,
                    } => match render_template(template_id, variables) {
                        Ok(prompt) => {
                            state.replace_input(prompt);
                            state.set_notice("Template rendered. Press Enter to send.".into());
                        }
                        Err(error) => state.set_notice(error),
                    },
                    UiEffect::Refresh(conversation_id) => match refresh(conversation_id) {
                        Ok(view) => state.apply_view(view),
                        Err(error) => state.set_notice(error),
                    },
                    UiEffect::ListDocuments {
                        workspace_id,
                        attach_id,
                    } => match list_documents(workspace_id) {
                        Ok(documents) => state.set_documents(documents, attach_id),
                        Err(error) => state.set_notice(error),
                    },
                    UiEffect::ListTemplates => match list_templates() {
                        Ok(templates) => state.set_templates(templates),
                        Err(error) => state.set_notice(error),
                    },
                    UiEffect::CreateSession(session) => match create_session(session) {
                        Ok(session) => state.add_session(session),
                        Err(error) => state.set_notice(error),
                    },
                    UiEffect::Login(provider) => {
                        terminal.suspend()?;
                        let result = login(provider);
                        terminal.resume()?;
                        match result {
                            Ok(notice) => state.set_notice(notice),
                            Err(error) => state.set_notice(error),
                        }
                    }
                    UiEffect::Continue => {}
                }
                if previous_thinking != state.show_thinking()
                    && let Err(error) = save_thinking(state.show_thinking())
                {
                    state.set_notice(error);
                }
            }
        }
        if last_live_refresh.elapsed() >= Duration::from_millis(250) {
            last_live_refresh = Instant::now();
            if state.awaiting_turn() || state.selected_status().is_some_and(status_is_live) {
                if let Some(conversation_id) =
                    state.selected().map(|item| item.conversation_id.clone())
                {
                    match refresh(conversation_id) {
                        Ok(view) => state.apply_view(view),
                        Err(error) => state.set_notice(error),
                    }
                }
            }
        }
    }
}
fn status_is_live(status: &gent_types::ConversationStatus) -> bool {
    status.runs.iter().any(|run| {
        run.active_turn_id.is_some()
            || run.live_status.as_ref().is_some_and(|live| {
                live.status.is_processing()
                    || live.status.needs_attention()
                    || live.status.is_waiting_for_subagents()
                    || live.status.has_live_subagent_work()
                    || live.status.is_waiting_for_command()
                    || live.status.has_live_command_work()
            })
    })
}
pub(crate) fn require_interactive() -> io::Result<()> {
    require_terminal(io::stdin().is_terminal(), io::stdout().is_terminal())
}
fn require_terminal(stdin_is_terminal: bool, stdout_is_terminal: bool) -> io::Result<()> {
    if stdin_is_terminal && stdout_is_terminal {
        return Ok(());
    }
    Err(io::Error::other(
        "interactive browser requires a terminal; use `gent conversation list`",
    ))
}
struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}
impl TerminalSession {
    fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                Err(error)
            }
        }
    }
    fn suspend(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
    }
    fn resume(&mut self) -> io::Result<()> {
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste
        )?;
        enable_raw_mode()?;
        self.terminal.clear()
    }
}
impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}
#[cfg(test)]
mod tests {
    use super::{require_terminal, status_is_live};
    use gent_types::{
        ConversationAttentionStatus, ConversationErrorStatus, ConversationLiveStatus,
        ConversationProcessingStatus, ConversationRunStatus, ConversationStatus,
        ConversationWorkStatus, HostEpoch, RunLiveStatus,
    };
    #[test]
    fn browser_requires_both_interactive_standard_streams() {
        assert!(require_terminal(true, true).is_ok());
        assert!(require_terminal(false, true).is_err());
        assert!(require_terminal(true, false).is_err());
    }
    #[test]
    fn live_status_refreshes_only_active_conversations() {
        let mut status = ConversationStatus {
            conversation_id: "conversation-1".into(),
            runs: vec![ConversationRunStatus {
                run_id: "run-1".into(),
                parent_run_id: None,
                provider: "claude".into(),
                active_turn_id: None,
                live_status: None,
            }],
        };
        assert!(!status_is_live(&status));
        status.runs[0].active_turn_id = Some("turn-1".into());
        assert!(status_is_live(&status));
        status.runs[0].active_turn_id = None;
        assert!(!status_is_live(&status));
        status.runs[0].live_status = Some(RunLiveStatus {
            run_id: "run-1".into(),
            host_epoch: HostEpoch(1),
            status: ConversationLiveStatus {
                cursor: 1,
                processing: ConversationProcessingStatus::Idle,
                attention: ConversationAttentionStatus::Required,
                error: ConversationErrorStatus::Clear,
                subagent_work: ConversationWorkStatus::None,
                command_work: ConversationWorkStatus::None,
            },
        });
        assert!(status_is_live(&status));
    }
}
