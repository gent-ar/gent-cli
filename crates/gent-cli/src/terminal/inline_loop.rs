use std::{
    collections::BTreeSet,
    io::{self, Stdout},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget, Wrap},
};

use super::{
    effects,
    inline_items::{ScrollItem, scroll_items, spaced},
    inline_view::compose,
    input,
    render::selected_title,
    state::{UiCommand, UiState},
    terminal_loop::{TerminalPorts, require_interactive},
};

struct InlineScreen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    height: u16,
}

impl InlineScreen {
    fn open() -> io::Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self {
            terminal: inline_terminal(1)?,
            height: 1,
        })
    }

    fn fit(&mut self, height: u16) -> io::Result<()> {
        if height != self.height {
            self.terminal.clear()?;
            self.terminal = inline_terminal(height)?;
            self.height = height;
        }
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        self.terminal.clear()?;
        self.terminal = inline_terminal(self.height)?;
        Ok(())
    }

    fn insert(&mut self, lines: Vec<Line<'static>>) -> io::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let width = self.terminal.size()?.width;
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let height = u16::try_from(paragraph.line_count(width)).unwrap_or(u16::MAX);
        self.terminal
            .insert_before(height, |buffer| paragraph.render(buffer.area, buffer))
    }
}

impl Drop for InlineScreen {
    fn drop(&mut self) {
        let _ = self.terminal.clear();
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = self.terminal.show_cursor();
    }
}

fn inline_terminal(height: u16) -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )
}

#[derive(Default)]
struct Scrollback {
    conversation_id: Option<String>,
    committed: BTreeSet<String>,
    previous_tool: Option<bool>,
}

impl Scrollback {
    fn commit(&mut self, state: &UiState, width: u16) -> (Vec<Line<'static>>, Vec<ScrollItem>) {
        let mut lines = Vec::new();
        let selected = state.selected().map(|item| item.conversation_id.clone());
        if selected != self.conversation_id {
            self.conversation_id.clone_from(&selected);
            self.committed.clear();
            self.previous_tool = None;
            lines.extend(header(state));
        }
        let mut finished = Vec::new();
        let mut pending = Vec::new();
        for item in scroll_items(state, width) {
            if self.committed.contains(&item.key) {
                continue;
            }
            if pending.is_empty() && item.done {
                finished.push(item);
            } else {
                pending.push(item);
            }
        }
        if !finished.is_empty() {
            lines.extend(spaced(
                &finished.iter().collect::<Vec<_>>(),
                self.previous_tool,
            ));
            self.previous_tool = finished.last().map(|item| item.tool);
            self.committed
                .extend(finished.into_iter().map(|item| item.key));
        }
        (lines, pending)
    }
}

fn header(state: &UiState) -> Vec<Line<'static>> {
    let workspace = state
        .selected_workspace_path()
        .map(|path| path.strip_prefix(r"\\?\").unwrap_or(path).to_owned())
        .unwrap_or_default();
    vec![
        Line::default(),
        Line::styled(
            format!(
                "✻ gent {} · {}",
                env!("CARGO_PKG_VERSION"),
                selected_title(state)
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            format!("  {workspace}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]
}

fn overlay_open(state: &UiState) -> bool {
    state.help_visible()
        || state.picker_view().is_some()
        || state.conversation_picker().is_some()
        || state.documents_visible
        || state.templates_visible
        || state.automations_visible
        || state.palette_open()
        || state.inline().bypass_confirm
        || (state.input().is_empty() && state.selected_pending_permission().is_some())
}

fn accepts(state: &UiState, command: &UiCommand) -> bool {
    match command {
        UiCommand::SelectNext | UiCommand::SelectPrevious => overlay_open(state),
        UiCommand::ScrollOlder
        | UiCommand::ScrollNewer
        | UiCommand::FollowLatest
        | UiCommand::ToggleActivity
        | UiCommand::FocusSessions => false,
        _ => true,
    }
}

pub(crate) fn run_inline(mut state: UiState, mut ports: TerminalPorts<'_>) -> io::Result<()> {
    require_interactive()?;
    let mut screen = InlineScreen::open()?;
    let mut scrollback = Scrollback::default();
    let mut clock = effects::LiveClock::new();
    let mut turn_started: Option<Instant> = None;
    effects::refresh_workspace_git(&mut state, &mut ports, false);
    loop {
        let size = screen.terminal.size()?;
        let (finished, pending) = scrollback.commit(&state, size.width);
        turn_started = state
            .turn_active()
            .then(|| turn_started.unwrap_or_else(Instant::now));
        let composition = compose(
            &state,
            &pending.iter().collect::<Vec<_>>(),
            scrollback.previous_tool,
            turn_started.map(|started| started.elapsed()),
            size.width,
        );
        screen.fit(composition.height(size.width, size.height * 2 / 3))?;
        screen.insert(finished)?;
        screen
            .terminal
            .draw(|frame| composition.draw(frame, &state))?;
        if event::poll(Duration::from_millis(100))? {
            let command = match event::read()? {
                Event::Key(key) => input::command(key, state.chat_enabled()),
                Event::Paste(value) if state.chat_enabled() => Some(UiCommand::Paste(value)),
                Event::Resize(_, _) => {
                    screen.reset()?;
                    None
                }
                _ => None,
            };
            if let Some(command) = command.filter(|command| accepts(&state, command)) {
                let quit =
                    effects::apply_command(&mut state, &mut ports, command, &mut |suspend| {
                        if suspend {
                            screen.terminal.clear()?;
                            disable_raw_mode()
                        } else {
                            enable_raw_mode()?;
                            screen.reset()
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
