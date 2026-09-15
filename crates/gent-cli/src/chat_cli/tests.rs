use gent_protocol::AgentChatIntentFrame;
use gent_types::{HostEpoch, Receipt, ReceiptStatus};

use super::{ChatCommand, CreateArgs, Mode, Provider, frame, valid_reply};

#[path = "tests/ipc_roundtrips.rs"]
mod ipc_roundtrips;

#[test]
fn selection_switch_carries_each_provider_model_effort_mode_and_context_policy() {
    let cases = [
        (
            Provider::Codex,
            gent_types::AgentChatEffort::Low,
            Mode::Ask,
            super::switch::Context::Preserve,
            gent_types::AgentChatProvider::Codex,
            gent_types::AgentChatEffort::Low,
            gent_types::AgentChatMode::Ask,
            gent_types::ContextPolicy::Preserve,
        ),
        (
            Provider::Claude,
            gent_types::AgentChatEffort::Medium,
            Mode::Plan,
            super::switch::Context::Clear,
            gent_types::AgentChatProvider::Claude,
            gent_types::AgentChatEffort::Medium,
            gent_types::AgentChatMode::Plan,
            gent_types::ContextPolicy::Clear,
        ),
        (
            Provider::Gent,
            gent_types::AgentChatEffort::High,
            Mode::Agent,
            super::switch::Context::Preserve,
            gent_types::AgentChatProvider::Claurst,
            gent_types::AgentChatEffort::High,
            gent_types::AgentChatMode::Agent,
            gent_types::ContextPolicy::Preserve,
        ),
    ];
    for (
        provider,
        effort,
        mode,
        context,
        expected_provider,
        expected_effort,
        expected_mode,
        expected_context,
    ) in cases
    {
        let request = frame(ChatCommand::Switch(super::switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: Some("run-1".into()),
            selection: crate::chat_cli::SelectionArgs {
                provider: Some(provider),
                model: Some("exact-model".into()),
                effort: Some(effort),
                mode: Some(mode),
            },
            context,
            request_id: Some("request-1".into()),
            receipt_id: Some("receipt-1".into()),
        }))
        .unwrap();
        let AgentChatIntentFrame::SwitchSelection {
            selection,
            context_policy,
            ..
        } = request
        else {
            panic!("expected selection switch");
        };
        assert_eq!(selection.provider, expected_provider);
        assert_eq!(selection.model, "exact-model");
        assert_eq!(selection.effort, expected_effort);
        assert_eq!(selection.mode, expected_mode);
        assert_eq!(context_policy, expected_context);
    }
}

#[test]
fn create_without_a_choice_leaves_the_selection_to_gentd() {
    let request = frame(ChatCommand::Create(CreateArgs {
        workspace: None,
        selection: crate::chat_cli::SelectionArgs {
            provider: None,
            model: None,
            effort: None,
            mode: None,
        },
        request_id: Some("request-1".into()),
        receipt_id: Some("receipt-1".into()),
    }))
    .unwrap();
    assert!(matches!(
        request,
        AgentChatIntentFrame::CreateConversation {
            selection: None,
            ..
        }
    ));
}

#[test]
fn clear_context_refuses_a_reply_that_claims_inherited_history() {
    let request = frame(ChatCommand::Switch(super::switch::SwitchArgs {
        conversation_id: "conversation-1".into(),
        parent_run_id: Some("run-1".into()),
        selection: crate::chat_cli::SelectionArgs {
            provider: Some(Provider::Claude),
            model: Some("sonnet".into()),
            effort: Some(gent_types::AgentChatEffort::Medium),
            mode: Some(Mode::Plan),
        },
        context: super::switch::Context::Clear,
        request_id: Some("request-1".into()),
        receipt_id: Some("receipt-1".into()),
    }))
    .unwrap();
    let AgentChatIntentFrame::SwitchSelection {
        request_id,
        receipt_id,
        conversation_id,
        parent_run_id,
        context_policy,
        ..
    } = request.clone()
    else {
        panic!("expected selection switch");
    };
    let reply = AgentChatIntentFrame::Switched {
        request_id,
        receipt: Receipt {
            receipt_id,
            idempotency_key: "retry-1".into(),
            status: ReceiptStatus::Settled,
            host_epoch: HostEpoch(1),
        },
        conversation_id,
        parent_run_id,
        run_id: gent_types::AgentChatRunId("run-2".into()),
        context_policy,
        context_through_ordinal: 1,
    };
    assert!(!valid_reply(&request, &reply));
}

#[path = "tests/switch_current.rs"]
mod switch_current;

#[path = "tests/accepted_prompt.rs"]
mod accepted_prompt;
