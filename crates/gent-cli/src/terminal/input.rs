//! Terminal event translation with no rendering or protocol dependencies.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::state::UiCommand;

#[must_use]
pub(crate) fn command(event: KeyEvent) -> Option<UiCommand> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    match event.code {
        KeyCode::Down | KeyCode::Char('j') => Some(UiCommand::SelectNext),
        KeyCode::Up | KeyCode::Char('k') => Some(UiCommand::SelectPrevious),
        KeyCode::Esc | KeyCode::Char('q') => Some(UiCommand::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::command;
    use crate::terminal::state::UiCommand;

    #[test]
    fn only_key_presses_become_typed_navigation_commands() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(UiCommand::SelectNext)
        );
        assert_eq!(
            command(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Release,
                state: KeyEventState::NONE,
            }),
            None
        );
    }
}
