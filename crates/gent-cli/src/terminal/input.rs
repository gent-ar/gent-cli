use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::state::UiCommand;

#[must_use]
pub(crate) fn command(event: KeyEvent, chat_enabled: bool) -> Option<UiCommand> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    match event.code {
        KeyCode::Down => Some(UiCommand::SelectNext),
        KeyCode::Char('j') if !chat_enabled => Some(UiCommand::SelectNext),
        KeyCode::Up => Some(UiCommand::SelectPrevious),
        KeyCode::PageUp => Some(UiCommand::ScrollOlder),
        KeyCode::PageDown => Some(UiCommand::ScrollNewer),
        KeyCode::Char('k') if !chat_enabled => Some(UiCommand::SelectPrevious),
        KeyCode::Esc => Some(UiCommand::Quit),
        KeyCode::Char('q') if !chat_enabled => Some(UiCommand::Quit),
        KeyCode::Char('q') if chat_enabled && event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::Quit)
        }
        KeyCode::Enter
            if event.modifiers.contains(KeyModifiers::SHIFT)
                || event.modifiers.contains(KeyModifiers::ALT) =>
        {
            Some(UiCommand::InsertNewline)
        }
        KeyCode::Enter => Some(UiCommand::SubmitPrompt),
        KeyCode::F(1) => Some(UiCommand::ToggleHelp),
        KeyCode::F(2) => Some(UiCommand::ToggleActivity),
        KeyCode::Backspace => Some(UiCommand::DeleteInput),
        KeyCode::Tab => Some(UiCommand::CycleProvider),
        KeyCode::Char('n') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CreateConversation)
        }
        KeyCode::Char('e') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CycleEffort)
        }
        KeyCode::Char('l') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CycleModel)
        }
        KeyCode::Char('o') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CycleMode)
        }
        KeyCode::Char('p') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CyclePermission)
        }
        KeyCode::Char('x') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::CycleContext)
        }
        KeyCode::Char('s') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::SwitchSelection)
        }
        KeyCode::Char('g') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::FocusSessions)
        }
        KeyCode::Char('t') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::ToggleThinking)
        }
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiCommand::Interrupt)
        }
        KeyCode::Char(value)
            if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT =>
        {
            Some(UiCommand::Insert(value))
        }
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
            command(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), false),
            Some(UiCommand::SelectNext)
        );
        assert_eq!(
            command(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                true
            ),
            Some(UiCommand::SwitchSelection)
        );
        assert_eq!(
            command(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE), true),
            Some(UiCommand::ToggleActivity)
        );
        assert_eq!(
            command(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                true
            ),
            Some(UiCommand::CycleMode)
        );
        assert_eq!(
            command(
                KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Release,
                    state: KeyEventState::NONE,
                },
                false
            ),
            None
        );
    }

    #[test]
    fn chat_mode_keeps_q_for_prompt_text_and_uses_control_q_to_quit() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), true),
            Some(UiCommand::Insert('q'))
        );
        assert_eq!(
            command(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                true
            ),
            Some(UiCommand::Quit)
        );
    }

    #[test]
    fn chat_mode_keeps_navigation_letters_for_prompt_text() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), true),
            Some(UiCommand::Insert('j'))
        );
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), true),
            Some(UiCommand::Insert('k'))
        );
    }

    #[test]
    fn chat_mode_accepts_shift_modified_prompt_characters() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT), true),
            Some(UiCommand::Insert('P'))
        );
        assert_eq!(
            command(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT), true),
            Some(UiCommand::Insert(':'))
        );
    }

    #[test]
    fn enter_is_never_a_mode_shortcut() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true),
            Some(UiCommand::SubmitPrompt)
        );
        assert_eq!(
            command(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                true
            ),
            Some(UiCommand::CycleMode)
        );
    }

    #[test]
    fn modified_enter_inserts_a_composer_newline() {
        assert_eq!(
            command(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), true),
            Some(UiCommand::InsertNewline)
        );
        assert_eq!(
            command(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), true),
            Some(UiCommand::InsertNewline)
        );
    }
}
