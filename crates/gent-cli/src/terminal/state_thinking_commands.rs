use super::{UiCommand, UiEffect, UiState};

pub(super) fn command(state: &mut UiState, argument: &str) -> UiEffect {
    match argument {
        "show" => {
            if !state.show_thinking() {
                return state.apply(UiCommand::ToggleThinking);
            }
        }
        "hide" => {
            if state.show_thinking() {
                return state.apply(UiCommand::ToggleThinking);
            }
        }
        "toggle" => return state.apply(UiCommand::ToggleThinking),
        _ => {
            state.set_notice("/thinking requires show, hide, or toggle.".into());
            return UiEffect::Continue;
        }
    }
    state.replace_input(String::new());
    state.set_notice("Thinking visibility is unchanged.".into());
    UiEffect::Continue
}

pub(super) fn activity(state: &mut UiState, argument: &str) -> UiEffect {
    if !argument.is_empty() {
        state.set_notice("/activity does not take an argument.".into());
        return UiEffect::Continue;
    }
    state.replace_input(String::new());
    state.apply(UiCommand::ToggleActivity)
}
