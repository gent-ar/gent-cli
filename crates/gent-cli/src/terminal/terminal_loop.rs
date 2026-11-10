use super::{
    effects, input, render,
    state::{UiCommand, UiRequest, UiRequestResult, UiState},
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
    time::Duration,
};
pub(crate) struct TerminalPorts<'a> {
    pub(crate) request: Box<dyn FnMut(UiRequest) -> Result<UiRequestResult, String> + 'a>,
    pub(crate) refresh: Box<dyn FnMut(String) -> Result<super::ConversationView, String> + 'a>,
    pub(crate) render_template:
        Box<dyn FnMut(String, Vec<PromptTemplateVariable>) -> Result<String, String> + 'a>,
    pub(crate) list_documents:
        Box<dyn FnMut(String) -> Result<Vec<gent_protocol::WorkspaceDocumentRecord>, String> + 'a>,
    pub(crate) list_templates:
        Box<dyn FnMut() -> Result<Vec<gent_types::PromptTemplateRecord>, String> + 'a>,
    pub(crate) create_session: Box<
        dyn FnMut(gent_types::AgentChatSession) -> Result<gent_types::AgentChatSession, String>
            + 'a,
    >,
    pub(crate) login: Box<dyn FnMut(gent_types::AgentChatProvider) -> Result<String, String> + 'a>,
    pub(crate) save_thinking: Box<dyn FnMut(bool) -> Result<(), String> + 'a>,
    pub(crate) workspace_git: Box<dyn FnMut(Option<String>) -> Option<WorkspaceGitResult> + 'a>,
    pub(crate) conversations: Box<dyn FnMut() -> Result<ConversationList, String> + 'a>,
}

pub(crate) type WorkspaceGitResult = (
    String,
    Result<Option<gent_types::WorkspaceGitReport>, String>,
);

pub(crate) type ConversationList = (
    Vec<gent_types::ConversationListItem>,
    std::collections::BTreeMap<String, super::ConversationMetadata>,
);

pub(crate) fn run(mut state: UiState, mut ports: TerminalPorts<'_>) -> io::Result<()> {
    require_interactive()?;
    let mut terminal = TerminalSession::open()?;
    let mut clock = effects::LiveClock::new();
    effects::refresh_workspace_git(&mut state, &mut ports, false);
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
                let quit =
                    effects::apply_command(&mut state, &mut ports, command, &mut |suspend| {
                        if suspend {
                            terminal.suspend()
                        } else {
                            terminal.resume()
                        }
                    })?;
                if quit {
                    return Ok(());
                }
            }
        }
        effects::live_tick(&mut state, &mut ports, &mut clock);
    }
}
pub(crate) fn status_is_live(status: &gent_types::ConversationStatus) -> bool {
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
