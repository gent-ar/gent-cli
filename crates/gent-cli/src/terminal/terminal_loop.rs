//! Terminal lifecycle edge: alternate screen, raw mode, event loop, and restoration.

use std::io::{self, IsTerminal, Stdout};

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use super::{input, render, state::UiState};

pub(crate) fn run(mut state: UiState) -> io::Result<()> {
    require_interactive()?;
    let mut terminal = TerminalSession::open()?;
    loop {
        terminal
            .terminal
            .draw(|frame| render::render(frame, &state))?;
        if let Event::Key(key) = event::read()? {
            if input::command(key).is_some_and(|command| state.apply(command)) {
                return Ok(());
            }
        }
    }
}

/// Rejects a browser launch before it opens a local IPC connection or terminal session.
///
/// # Errors
/// Returns an error unless both standard streams are interactive terminals.
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
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
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
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::require_terminal;

    #[test]
    fn browser_requires_both_interactive_standard_streams() {
        assert!(require_terminal(true, true).is_ok());
        assert!(require_terminal(false, true).is_err());
        assert!(require_terminal(true, false).is_err());
    }
}
