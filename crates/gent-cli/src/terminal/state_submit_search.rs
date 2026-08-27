use super::{UiEffect, UiState, notices::notice};

pub(super) fn command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if argument == "clear" {
        let selected = state.set_conversation_filter("");
        state.input.clear();
        state.notice = Some("Conversation filter cleared.".into());
        return Some(selected.map_or(UiEffect::Continue, UiEffect::Refresh));
    }
    if argument.is_empty() {
        return Some(notice(
            state,
            "/search TEXT filters titles and recaps; /search clear resets.",
        ));
    }
    let selected = state.set_conversation_filter(argument);
    state.input.clear();
    state.notice = Some(format!("Showing conversations matching {argument:?}."));
    Some(selected.map_or(UiEffect::Continue, UiEffect::Refresh))
}
