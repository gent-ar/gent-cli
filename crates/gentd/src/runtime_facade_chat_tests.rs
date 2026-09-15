//! Integration checks for durable-chat read composition, separate from observer tests.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY, AgentChatConversationFrame, AgentChatIntentFrame,
    AgentChatTranscriptFrame, HistoricalTranscriptEntry, ORCHESTRATION_CAPABILITY,
    PROVIDER_READINESS_CAPABILITY, REVIEWED_PLAN_CAPABILITY,
};
use gent_runtime::catalog::{
    RuntimeCapabilityFeature, RuntimeCapabilityProfile, declared_capabilities_with_profiles,
    validate_observed_capabilities,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ContextPolicy, NormalizedTranscriptKind, ReceiptId,
};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn durable_chat_authority_advertises_and_serves_only_normalized_read_models() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]);
    let capabilities = crate::transport::observed_capabilities(&profile);
    assert_eq!(
        validate_observed_capabilities(&capabilities).unwrap(),
        declared_capabilities_with_profiles(&profile)
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_CONVERSATIONS_CAPABILITY.into())
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_TRANSCRIPT_CAPABILITY.into())
    );
    assert!(
        capabilities
            .0
            .contains(&AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY.into())
    );
    assert!(capabilities.0.contains(&ORCHESTRATION_CAPABILITY.into()));
    assert!(!capabilities.0.contains(&REVIEWED_PLAN_CAPABILITY.into()));
    assert!(
        !capabilities
            .0
            .contains(&PROVIDER_READINESS_CAPABILITY.into())
    );
    let runtime = build_runtime(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
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
        panic!("durable chat profile must create one conversation");
    };
    let summary = runtime
        .agent_chat_conversation(AgentChatConversationFrame::SummaryRequest {
            conversation_id: conversation_id.0.clone(),
        })
        .unwrap();
    assert!(matches!(
        summary,
        AgentChatConversationFrame::Summary(value)
            if value.selection.provider == AgentChatProvider::Codex
    ));
    let transcript = runtime
        .agent_chat_transcript(AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.0.clone(),
            after_cursor: None,
            limit: 20,
        })
        .unwrap();
    assert!(matches!(
        transcript,
        AgentChatTranscriptFrame::Page(value)
            if value.conversation_id == conversation_id.0 && value.events.is_empty()
    ));
}

#[test]
fn selection_switch_rejects_an_unsettled_parent_turn() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]);
    let runtime = build_runtime(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create-mid-turn".into()),
            receipt_id: ReceiptId("receipt-create-mid-turn".into()),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "sonnet".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Ask,
            }),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("conversation creation must return durable identities");
    };
    runtime
        .agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt-mid-turn".into()),
            receipt_id: ReceiptId("receipt-prompt-mid-turn".into()),
            conversation_id: conversation_id.clone(),
            text: "continue".into(),
            attachment_ids: vec![],
        })
        .unwrap();
    let error = runtime
        .agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("switch-mid-turn".into()),
            receipt_id: ReceiptId("receipt-switch-mid-turn".into()),
            conversation_id: conversation_id.clone(),
            parent_run_id: run_id.clone(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6-sol".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap_err();
    assert_eq!(error.code, "selectionSwitchBlockedByActiveTurn");
    assert_eq!(
        error.message,
        "the current turn must settle before changing its model or provider"
    );
}

#[test]
fn historical_import_is_durable_idempotent_and_precedes_live_prompts() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]);
    let runtime = build_runtime(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create-history".into()),
            receipt_id: ReceiptId("receipt-history".into()),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "sonnet".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            }),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("conversation creation must return durable identities");
    };
    let entries = vec![
        HistoricalTranscriptEntry {
            source_id: "legacy-user-1".into(),
            kind: NormalizedTranscriptKind::UserMessage,
            text: "Explain this repository.".into(),
        },
        HistoricalTranscriptEntry {
            source_id: "legacy-assistant-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "It is a Flutter app.".into(),
        },
    ];
    for request_id in ["import-history", "retry-history"] {
        let result = runtime
            .agent_chat_intent(AgentChatIntentFrame::ImportTranscript {
                request_id: AgentChatRequestId(request_id.into()),
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
                entries: entries.clone(),
            })
            .unwrap();
        assert!(matches!(
            result.as_slice(),
            [AgentChatIntentFrame::TranscriptImported {
                imported_count: 2,
                ..
            }]
        ));
    }
    let transcript = runtime
        .agent_chat_transcript(AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.0.clone(),
            after_cursor: None,
            limit: 20,
        })
        .unwrap();
    assert!(matches!(
        transcript,
        AgentChatTranscriptFrame::Page(value)
            if value.events.len() == 2
                && value.events[0].kind == NormalizedTranscriptKind::UserMessage
                && value.events[1].kind == NormalizedTranscriptKind::AssistantMessage
    ));
}

#[test]
fn readiness_profile_cannot_advertise_without_exact_private_authority() {
    let directory = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderReadiness,
    ]);
    assert!(
        build_runtime(
            directory.path(),
            &profile,
            CompatibilityAssessment::default()
        )
        .is_err()
    );
}

fn chat_runtime(directory: &std::path::Path) -> crate::runtime_facade::RuntimeFacade {
    build_runtime(
        directory,
        &RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat,
            RuntimeCapabilityFeature::AgentChatProjection,
        ]),
        CompatibilityAssessment::default(),
    )
    .unwrap()
}

fn create_conversation(
    runtime: &crate::runtime_facade::RuntimeFacade,
    key: &str,
) -> (
    gent_types::AgentChatConversationId,
    gent_types::AgentChatRunId,
) {
    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(format!("create-{key}")),
            receipt_id: ReceiptId(format!("receipt-create-{key}")),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "sonnet".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            }),
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("conversation creation must return durable identities");
    };
    (conversation_id.clone(), run_id.clone())
}

fn prompt_message(
    runtime: &crate::runtime_facade::RuntimeFacade,
    frame: AgentChatIntentFrame,
) -> String {
    let accepted = runtime.agent_chat_intent(frame).unwrap();
    let [AgentChatIntentFrame::Accepted { message_id, .. }] = accepted.as_slice() else {
        panic!("prompt must be accepted");
    };
    message_id.clone()
}

#[test]
fn queued_prompt_cancellation_is_durable_idempotent_and_typed_when_not_queued() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = chat_runtime(directory.path());
    let (conversation_id, _) = create_conversation(&runtime, "cancel");
    let sent = prompt_message(
        &runtime,
        AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("send".into()),
            receipt_id: ReceiptId("receipt-send".into()),
            conversation_id: conversation_id.clone(),
            text: "first".into(),
            attachment_ids: vec![],
        },
    );
    let queued = prompt_message(
        &runtime,
        AgentChatIntentFrame::QueuePromptWithTools {
            request_id: AgentChatRequestId("queued".into()),
            receipt_id: ReceiptId("receipt-queued".into()),
            conversation_id: conversation_id.clone(),
            text: "later".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        },
    );
    let cancel = |request: &str, receipt: &str, message: &str| {
        runtime.agent_chat_intent(AgentChatIntentFrame::CancelQueuedPrompt {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(receipt.into()),
            conversation_id: conversation_id.clone(),
            message_id: message.into(),
        })
    };
    assert_eq!(
        cancel("cancel-sent", "receipt-cancel-sent", &sent)
            .unwrap_err()
            .code,
        "queuedPromptNotCancelable"
    );
    let first = cancel("cancel-1", "receipt-cancel", &queued).unwrap();
    let [
        AgentChatIntentFrame::QueuedPromptCanceled {
            receipt,
            message_id,
            ..
        },
    ] = first.as_slice()
    else {
        panic!("queued prompt must be canceled");
    };
    assert_eq!(message_id, &queued);
    let retry = cancel("cancel-retry", "receipt-cancel", &queued).unwrap();
    assert!(matches!(
        retry.as_slice(),
        [AgentChatIntentFrame::QueuedPromptCanceled { receipt: retried, request_id, .. }]
            if retried == receipt && request_id.0 == "cancel-retry"
    ));
    assert_eq!(
        cancel("cancel-2", "receipt-cancel-2", &queued)
            .unwrap_err()
            .code,
        "queuedPromptNotCancelable"
    );
    let snapshot = runtime
        .agent_chat_projection(
            gent_protocol::AgentChatProjectionFrame::ConversationSnapshotRequest {
                request_id: "snapshot".into(),
                conversation_id: conversation_id.0.clone(),
                transcript_limit: 100,
                activity_limit: 100,
            },
        )
        .unwrap();
    let gent_protocol::AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } = snapshot
    else {
        panic!("snapshot request must return a snapshot");
    };
    let queue_facts = snapshot
        .activity
        .iter()
        .filter_map(|fact| match fact {
            gent_types::ConversationActivityFact::PromptQueued { message_id, .. } => {
                Some(("queued", message_id))
            }
            gent_types::ConversationActivityFact::PromptCanceled { message_id, .. } => {
                Some(("canceled", message_id))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(queue_facts, [("queued", &queued), ("canceled", &queued)]);
}

#[test]
fn steering_without_a_provider_lifecycle_leaves_the_queue_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = chat_runtime(directory.path());
    let (conversation_id, _) = create_conversation(&runtime, "steer");
    let queued = prompt_message(
        &runtime,
        AgentChatIntentFrame::QueuePrompt {
            request_id: AgentChatRequestId("queued".into()),
            receipt_id: ReceiptId("receipt-queued".into()),
            conversation_id: conversation_id.clone(),
            text: "later".into(),
            attachment_ids: vec![],
        },
    );
    let steer = runtime.agent_chat_intent(AgentChatIntentFrame::SteerQueuedPrompt {
        request_id: AgentChatRequestId("steer".into()),
        receipt_id: ReceiptId("receipt-steer".into()),
        conversation_id: conversation_id.clone(),
        message_id: queued.clone(),
    });
    assert!(steer.is_err());
    assert!(matches!(
        runtime
            .agent_chat_intent(AgentChatIntentFrame::CancelQueuedPrompt {
                request_id: AgentChatRequestId("cancel".into()),
                receipt_id: ReceiptId("receipt-steer".into()),
                conversation_id,
                message_id: queued.clone(),
            })
            .unwrap()
            .as_slice(),
        [AgentChatIntentFrame::QueuedPromptCanceled { message_id, .. }] if *message_id == queued
    ));
}

#[test]
fn switching_from_a_superseded_parent_is_a_typed_rejection() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = chat_runtime(directory.path());
    let (conversation_id, root) = create_conversation(&runtime, "switch");
    let switch = |request: &str, parent: &gent_types::AgentChatRunId| {
        runtime.agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId(request.into()),
            receipt_id: ReceiptId(format!("receipt-{request}")),
            conversation_id: conversation_id.clone(),
            parent_run_id: parent.clone(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Agent,
            },
            context_policy: ContextPolicy::Preserve,
        })
    };
    assert!(matches!(
        switch("switch-1", &root).unwrap().as_slice(),
        [AgentChatIntentFrame::Switched { .. }]
    ));
    let error = switch("switch-2", &root).unwrap_err();
    assert_eq!(error.code, "selectionSwitchParentNotCurrent");
    assert_eq!(
        error.message,
        "the selection can change only from the conversation's current run"
    );
}
