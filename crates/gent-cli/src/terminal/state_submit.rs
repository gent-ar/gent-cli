//! Prompt and `/goal` request construction for the pure terminal reducer.

use super::{UiEffect, UiRequest, UiState};

pub(super) fn submit(state: &mut UiState) -> UiEffect {
    let Some(conversation_id) = state.selected().map(|value| value.conversation_id.clone()) else {
        state.notice = Some("Create a conversation first with Ctrl+N.".into());
        return UiEffect::Continue;
    };
    let text = state.input.trim().to_owned();
    if text.is_empty() {
        return UiEffect::Continue;
    }
    match goal_summary(&text) {
        Ok(Some(summary)) => {
            let Some(run_id) = state.parent_run_id.clone() else {
                state.notice =
                    Some("Run status is unavailable; refusing to guess a /goal binding.".into());
                return UiEffect::Continue;
            };
            state.input.clear();
            return UiEffect::Request(UiRequest::Goal {
                conversation_id,
                run_id,
                summary,
            });
        }
        Ok(None) => {}
        Err(notice) => {
            state.notice = Some(notice.into());
            return UiEffect::Continue;
        }
    }
    state.input.clear();
    UiEffect::Request(UiRequest::Send {
        conversation_id,
        text,
    })
}

fn goal_summary(text: &str) -> Result<Option<String>, &'static str> {
    let Some(summary) = text.strip_prefix("/goal") else {
        return Ok(None);
    };
    if !summary.is_empty() && !summary.chars().next().is_some_and(char::is_whitespace) {
        return Ok(None);
    }
    let summary = summary.trim();
    (!summary.is_empty())
        .then(|| Some(summary.to_owned()))
        .ok_or("`/goal` requires a concise summary; no provider work was started")
}
