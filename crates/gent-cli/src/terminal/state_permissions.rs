use super::{UiEffect, UiRequest, UiState};

pub(super) fn settings_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    let (mode, bypass_consent) = match argument {
        "ask" => (gent_types::PermissionMode::Default, false),
        "read" => (gent_types::PermissionMode::Plan, false),
        "edits" => (gent_types::PermissionMode::AutoAcceptEdits, false),
        "autonomous" => (gent_types::PermissionMode::Autonomous, false),
        "bypass confirm" => (gent_types::PermissionMode::Bypass, true),
        _ => {
            state.notice = Some(
                "/permissions requires ask, read, edits, autonomous, or bypass confirm.".into(),
            );
            return Some(UiEffect::Continue);
        }
    };
    let Some(conversation_id) = state.selected().map(|item| item.conversation_id.clone()) else {
        state.notice = Some("Select a conversation before changing permissions.".into());
        return Some(UiEffect::Continue);
    };
    let Some(workspace_id) = state.selected_workspace_id().map(str::to_owned) else {
        state.notice =
            Some("Workspace details are unavailable; refresh the conversation first.".into());
        return Some(UiEffect::Continue);
    };
    state.input.clear();
    Some(UiEffect::Request(UiRequest::SetPermissionMode {
        conversation_id,
        workspace_id,
        mode,
        bypass_consent,
    }))
}

pub(super) fn command(state: &mut UiState, command: &str, argument: &str) -> Option<UiEffect> {
    if command != "/answer" && !argument.is_empty() {
        return None;
    }
    let input = if command == "/answer" {
        match serde_json::from_str(argument) {
            Ok(value) => Some(value),
            Err(_) => {
                state.notice =
                    Some("/answer requires a JSON object with the requested answers.".into());
                return Some(UiEffect::Continue);
            }
        }
    } else {
        None
    };
    let Some(request) = state.selected_pending_permission() else {
        state.notice = Some("There is no pending permission for this conversation.".into());
        return Some(UiEffect::Continue);
    };
    let response = gent_types::PermissionDecisionResponse {
        binding: request.binding.clone(),
        response: match command {
            "/deny" => gent_types::PermissionDecisionResponseKind::Deny,
            "/approve-tool" => gent_types::PermissionDecisionResponseKind::ApproveExactTool,
            "/approve-category" => gent_types::PermissionDecisionResponseKind::ApproveCategory,
            _ => gent_types::PermissionDecisionResponseKind::ApproveOnce,
        },
        input,
    };
    state.input.clear();
    Some(UiEffect::Request(UiRequest::Permission { response }))
}
