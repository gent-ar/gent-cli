use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, ContextPolicy};

use super::{UiEffect, UiState, notices::notice};
use crate::terminal::selection::default_model;

pub(super) fn command(state: &mut UiState, command: &str, argument: &str) -> Option<UiEffect> {
    match command {
        "/provider" => Some(provider(state, argument)),
        "/model" => Some(model(state, argument)),
        "/effort" => Some(effort(state, argument)),
        "/mode" | "/plan" => Some(mode(state, command, argument)),
        "/context" => Some(context(state, argument)),
        _ => None,
    }
}

fn provider(state: &mut UiState, argument: &str) -> UiEffect {
    let provider = match argument {
        "claude" => AgentChatProvider::Claude,
        "codex" => AgentChatProvider::Codex,
        "claurst" => AgentChatProvider::Claurst,
        _ => return notice(state, "/provider requires claude, codex, or claurst."),
    };
    state.selection.provider = provider;
    state.selection.model = default_model(provider).into();
    state.input.clear();
    apply_selection(state)
}

fn model(state: &mut UiState, argument: &str) -> UiEffect {
    if argument.is_empty() {
        return notice(state, "/model requires a model identifier.");
    }
    if !super::state_picker::model_options(state)
        .iter()
        .any(|model| model == argument)
    {
        state.input.clear();
        return notice(
            state,
            "That model is not available for the selected provider. Use Ctrl+L to choose one.",
        );
    }
    argument.clone_into(&mut state.selection.model);
    state.input.clear();
    apply_selection(state)
}

fn effort(state: &mut UiState, argument: &str) -> UiEffect {
    let effort = match argument {
        "low" => AgentChatEffort::Low,
        "medium" => AgentChatEffort::Medium,
        "high" => AgentChatEffort::High,
        "xhigh" => AgentChatEffort::XHigh,
        "max" => AgentChatEffort::Max,
        "ultra" => AgentChatEffort::Ultra,
        _ => {
            return notice(
                state,
                "/effort requires low, medium, high, xhigh, max, or ultra.",
            );
        }
    };
    if !super::state_picker::effort_options(state)
        .iter()
        .any(|value| value.eq_ignore_ascii_case(argument))
    {
        state.input.clear();
        return notice(
            state,
            "That effort level is not available for the selected provider. Use Ctrl+E to choose one.",
        );
    }
    state.selection.effort = effort;
    state.input.clear();
    apply_selection(state)
}

fn mode(state: &mut UiState, command: &str, argument: &str) -> UiEffect {
    let mode = if command == "/plan" && argument.is_empty() {
        AgentChatMode::Plan
    } else {
        match argument {
            "ask" => AgentChatMode::Ask,
            "plan" => AgentChatMode::Plan,
            "agent" => AgentChatMode::Agent,
            _ => return notice(state, "/mode requires ask, plan, or agent."),
        }
    };
    state.selection.mode = mode;
    state.input.clear();
    apply_selection(state)
}

fn context(state: &mut UiState, argument: &str) -> UiEffect {
    state.context_policy = match argument {
        "preserve" => ContextPolicy::Preserve,
        "clear" => ContextPolicy::Clear,
        _ => return notice(state, "/context requires preserve or clear."),
    };
    state.input.clear();
    state.notice = Some(format!(
        "Context selected: {:?}. Use /switch to apply it.",
        state.context_policy
    ));
    UiEffect::Continue
}

fn apply_selection(state: &mut UiState) -> UiEffect {
    match super::super::state_switch::request(
        state.selected().map(|item| item.conversation_id.clone()),
        state.parent_run_id.clone(),
        state.selection.clone(),
        state.context_policy,
    ) {
        Ok(effect) => effect,
        Err(_) => {
            state.notice = Some("Selection is ready for the next new conversation.".into());
            UiEffect::Continue
        }
    }
}
