use super::{UiEffect, UiState};

pub(crate) fn notice(state: &mut UiState, message: &str) -> UiEffect {
    state.notice = Some(message.into());
    UiEffect::Continue
}
