use gent_protocol::AgentChatIntentFrame;
use gent_types::{HostEpoch, Receipt, ReceiptStatus};

use super::{ChatCommand, CreateArgs, Effort, Mode, Provider, frame, valid_reply};

#[path = "tests/ipc_roundtrips.rs"]
mod ipc_roundtrips;

#[test]
fn selection_switch_carries_each_provider_model_effort_mode_and_context_policy() {
    let cases = [
        (
            Provider::Codex,
            Effort::Low,
            Mode::Ask,
            super::switch::Context::Preserve,
            gent_types::AgentChatProvider::Codex,
            gent_types::AgentChatEffort::Low,
            gent_types::AgentChatMode::Ask,
            gent_types::ContextPolicy::Preserve,
        ),
        (
            Provider::Claude,
            Effort::Medium,
            Mode::Plan,
            super::switch::Context::Clear,
            gent_types::AgentChatProvider::Claude,
            gent_types::AgentChatEffort::Medium,
            gent_types::AgentChatMode::Plan,
            gent_types::ContextPolicy::Clear,
        ),
        (
            Provider::Claurst,
            Effort::High,
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
            provider,
            model: "exact-model".into(),
            effort,
            mode,
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
fn claurst_default_model_is_a_shipped_curated_model() {
    let request = frame(ChatCommand::Create(CreateArgs {
        workspace: None,
        provider: Provider::Claurst,
        model: "default".into(),
        effort: Effort::Medium,
        mode: Mode::Agent,
        request_id: Some("request-1".into()),
        receipt_id: Some("receipt-1".into()),
    }))
    .unwrap();
    let AgentChatIntentFrame::CreateConversation { selection, .. } = request else {
        panic!("expected create");
    };
    assert_eq!(selection.model, gent_protocol::DEFAULT_LOCAL_MODEL_ID);
}

#[test]
fn clear_context_refuses_a_reply_that_claims_inherited_history() {
    let request = frame(ChatCommand::Switch(super::switch::SwitchArgs {
        conversation_id: "conversation-1".into(),
        parent_run_id: Some("run-1".into()),
        provider: Provider::Claude,
        model: "sonnet".into(),
        effort: Effort::Medium,
        mode: Mode::Plan,
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
