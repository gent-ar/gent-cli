use gent_types::ContextPolicy;

use super::{UiEffect, UiState, notices::notice};

pub(super) fn command(state: &mut UiState, command: &str, argument: &str) -> Option<UiEffect> {
    (command == "/context").then(|| context(state, argument))
}

pub(super) fn fit_effort(state: &mut UiState) {
    state.selection.effort = crate::chat_cli::fitted_effort(
        state.model_catalog.as_ref(),
        state.selection.provider,
        &state.selection.model,
        Some(state.selection.effort),
    );
}

fn context(state: &mut UiState, argument: &str) -> UiEffect {
    state.context_policy = match argument {
        "preserve" => ContextPolicy::Preserve,
        "clear" => ContextPolicy::Clear,
        _ => return notice(state, "/context-policy requires preserve or clear."),
    };
    state.input.clear();
    state.notice = Some(format!(
        "Context selected: {:?}. Use /switch to apply it.",
        state.context_policy
    ));
    UiEffect::Continue
}
