use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatReadLedger, AttachmentLedger,
    ConversationPromptLedger, Ledger, NormalizedSessionBatchLedger, RunLease, RunSessionBinding,
};
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, AttachmentMetadata, AttachmentState, AttachmentTransfer, DurableTurnPhase,
    HostEpoch, NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle,
    ProviderFailureClassification, ReceiptId,
};

use super::RuntimeFacade;
use crate::{CompatibilityAssessment, api::RuntimeApi, runtime_facade::DaemonCompositionState};

fn runtime(directory: &std::path::Path) -> (RuntimeFacade, SqliteLedger) {
    let state = DaemonCompositionState::open(
        directory,
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let ledger = state.ledger().clone();
    (RuntimeFacade::from_state(state, None).unwrap(), ledger)
}

fn conversation(runtime: &RuntimeFacade, key: &str) -> AgentChatConversationId {
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(format!("create-{key}")),
            receipt_id: ReceiptId(format!("create-receipt-{key}")),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            }),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id, ..
        },
    ] = created.as_slice()
    else {
        panic!("conversation creation must return its identity");
    };
    conversation_id.clone()
}

fn available_attachment(ledger: &SqliteLedger, attachment_id: &str) {
    let uploading = AttachmentTransfer {
        metadata: AttachmentMetadata {
            attachment_id: attachment_id.into(),
            display_name: "notes.txt".into(),
            media_type: "text/plain".into(),
            byte_len: 3,
            digest_sha256: "a".repeat(64),
            storage_key: format!("sha256/{attachment_id}"),
        },
        staging_key: format!("staging/{attachment_id}"),
        receipt_id: ReceiptId(format!("receipt-{attachment_id}")),
        idempotency_key: format!("key-{attachment_id}"),
        host_epoch: HostEpoch(1),
        state: AttachmentState::Uploading,
        received_bytes: 0,
    };
    ledger.claim_attachment(&uploading).unwrap();
    let mut uploaded = uploading.clone();
    uploaded.received_bytes = 3;
    ledger.replace_attachment(&uploading, &uploaded).unwrap();
    let mut available = uploaded.clone();
    available.state = AttachmentState::Available;
    ledger.replace_attachment(&uploaded, &available).unwrap();
}

fn failed_prompt(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    text: &str,
    attachment_ids: &[&str],
    classification: ProviderFailureClassification,
    delivery_confirmed: bool,
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("prompt-{text}")),
            receipt_id: ReceiptId(format!("prompt-receipt-{text}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: text.into(),
            attachment_ids: attachment_ids.iter().map(|id| (*id).into()).collect(),
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    let message_id = &saved.message.message_id;
    let run_id = &saved.run_id.0;
    ledger
        .claim_run_lease(&RunLease {
            run_id: run_id.clone(),
            coordinator_id: "daemon".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&RunSessionBinding {
            run_id: run_id.clone(),
            provider_session_id: format!("thread-{run_id}"),
        })
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(message_id, "daemon", HostEpoch(1))
        .unwrap();
    if delivery_confirmed {
        ledger
            .confirm_agent_chat_prompt_started(message_id, "daemon", HostEpoch(1))
            .unwrap();
    }
    ledger
        .append_normalized_session_batch(&NormalizedSessionBatch {
            coordinator_id: "daemon".into(),
            conversation_id: conversation_id.0.clone(),
            run_id: run_id.clone(),
            turn_id: saved.message.turn_id.clone(),
            host_epoch: HostEpoch(1),
            lifecycle_event_id: format!("failure-{message_id}"),
            lifecycle: NormalizedSessionLifecycle::Event {
                event: NormalizedProviderEvent::ProviderFailure {
                    classification,
                    message: gent_types::PROVIDER_SESSION_UNAVAILABLE_NOTICE.into(),
                },
            },
            transcript: None,
            activity_event_id: None,
            activity: None,
        })
        .unwrap();
    if delivery_confirmed {
        ledger
            .settle_agent_chat_prompt_terminal(
                message_id,
                "daemon",
                HostEpoch(1),
                DurableTurnPhase::Failed,
            )
            .unwrap();
    } else {
        ledger
            .mark_agent_chat_prompt_unprovable(message_id, "daemon", HostEpoch(1))
            .unwrap();
    }
    saved
}

fn continuation(
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    key: &str,
) -> AgentChatIntentFrame {
    AgentChatIntentFrame::ContinueFromSavedHistory {
        request_id: AgentChatRequestId(format!("continue-{key}")),
        receipt_id: ReceiptId(format!("continue-receipt-{key}")),
        conversation_id: conversation_id.clone(),
        message_id: message_id.into(),
    }
}

#[test]
fn continuing_an_unavailable_session_switches_once_and_resends_the_prompt_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let (runtime, ledger) = runtime(directory.path());
    let conversation_id = conversation(&runtime, "lost");
    available_attachment(&ledger, "attachment-notes");
    let failed = failed_prompt(
        &ledger,
        &conversation_id,
        "Which code did I give you?",
        &["attachment-notes"],
        ProviderFailureClassification::SessionUnavailable,
        true,
    );
    let frame = continuation(&conversation_id, &failed.message.message_id, "once");

    let first = runtime.agent_chat_intent(frame.clone()).unwrap();
    let retried = runtime.agent_chat_intent(frame).unwrap();

    assert_eq!(first, retried);
    let [
        AgentChatIntentFrame::Accepted {
            receipt,
            run_id,
            message_id,
            ..
        },
    ] = first.as_slice()
    else {
        panic!("continuation must accept exactly one prompt: {first:?}");
    };
    assert_eq!(receipt.receipt_id.0, "continue-receipt-once");
    assert_ne!(run_id, &failed.run_id);
    let detail = ledger.read_agent_chat_detail(&conversation_id.0).unwrap();
    assert_eq!(detail.current_run_id, run_id.0);
    assert_eq!(detail.runs.len(), 2);
    let child = detail
        .runs
        .iter()
        .find(|run| run.run_id == run_id.0)
        .unwrap();
    assert_eq!(
        child.parent_run_id.as_deref(),
        Some(failed.run_id.0.as_str())
    );
    assert_eq!(
        child.selection,
        detail
            .runs
            .iter()
            .find(|run| run.run_id == failed.run_id.0)
            .unwrap()
            .selection
    );
    let resent = ledger.list_run_messages(&run_id.0).unwrap();
    assert_eq!(resent.len(), 1);
    assert_eq!(resent[0].message_id, *message_id);
    assert_eq!(resent[0].text, failed.message.text);
    assert_eq!(
        ledger
            .turn_attachments(&resent[0].turn_id)
            .unwrap()
            .into_iter()
            .map(|attachment| attachment.attachment_id)
            .collect::<Vec<_>>(),
        ["attachment-notes"]
    );
    assert_eq!(ledger.list_run_messages(&failed.run_id.0).unwrap().len(), 1);
}

#[test]
fn only_a_turn_whose_provider_session_is_unavailable_can_continue_from_saved_history() {
    let directory = tempfile::tempdir().unwrap();
    let (runtime, ledger) = runtime(directory.path());
    for (key, delivery_confirmed) in [("genuine", true), ("unprovable", false)] {
        let conversation_id = conversation(&runtime, key);
        let failed = failed_prompt(
            &ledger,
            &conversation_id,
            key,
            &[],
            ProviderFailureClassification::Provider,
            delivery_confirmed,
        );

        let rejected = runtime
            .agent_chat_intent(continuation(
                &conversation_id,
                &failed.message.message_id,
                key,
            ))
            .unwrap_err();

        assert!(
            rejected.message.contains("provider session is unavailable"),
            "{key}"
        );
        let detail = ledger.read_agent_chat_detail(&conversation_id.0).unwrap();
        assert_eq!(detail.runs.len(), 1, "{key}");
    }
}
