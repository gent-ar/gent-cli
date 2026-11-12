use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatSelectionLedger,
    AgentChatWorkspaceLedger, TranscriptLedger,
};
use gent_runtime::{AgentChatRunContextService, ConversationContextArtifactService};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch,
    ContextPolicy, DurableTurnPhase, FrozenConversationContext, HostEpoch,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

const EPOCH: HostEpoch = HostEpoch(1);
const DAEMON: &str = "daemon-a";

fn selection(provider: AgentChatProvider, model: &str) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Ask,
    }
}

fn save(
    ledger: &SqliteLedger,
    conversation: &AgentChatConversationId,
    text: &str,
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(text.into()),
            receipt_id: ReceiptId(format!("receipt-{text}")),
            host_epoch: EPOCH,
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: text.into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(&saved.message.message_id, &saved.run_id, EPOCH)
        .unwrap();
    saved
}

fn next_turn_context(policy: ContextPolicy) -> FrozenConversationContext {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: EPOCH,
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run-1".into()),
                selection: selection(AgentChatProvider::Claude, "haiku"),
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let running = save(&ledger, &conversation, "first question");
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, EPOCH, AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&claimed.message.message_id, DAEMON, EPOCH)
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&claimed.message.message_id, DAEMON, EPOCH)
        .unwrap();
    let child = AgentChatRunId("run-2".into());
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: EPOCH,
            conversation_id: conversation.clone(),
            parent_run_id: AgentChatRunId("run-1".into()),
            run_id: child.clone(),
            selection: selection(AgentChatProvider::Codex, "gpt-5.6-luna"),
            context_policy: policy,
        })
        .unwrap();
    ledger
        .append_normalized_transcript(
            &conversation,
            &NormalizedTranscriptAppend {
                event_id: "reply-after-switch".into(),
                turn_id: running.message.turn_id.clone(),
                run_id: running.run_id.0.clone(),
                kind: NormalizedTranscriptKind::AssistantMessage,
                text: "reply finished on the old selection".into(),
                is_partial: false,
            },
        )
        .unwrap();
    ledger
        .settle_agent_chat_prompt_terminal(
            &running.message.message_id,
            DAEMON,
            EPOCH,
            DurableTurnPhase::Completed,
        )
        .unwrap();
    let follow_up = save(&ledger, &conversation, "follow-up");
    assert_eq!(follow_up.run_id, child);
    let context = AgentChatRunContextService::new(ledger.clone())
        .resolve(&conversation, &child)
        .unwrap();
    ConversationContextArtifactService::new(ledger)
        .project_run_before_message(&context, &follow_up.message.message_id)
        .unwrap()
}

#[test]
fn a_preserving_switch_mid_turn_gives_the_next_turn_the_finished_reply() {
    let context = next_turn_context(ContextPolicy::Preserve);
    assert_eq!(
        context
            .entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>(),
        ["first question"]
    );
    assert_eq!(
        context
            .transcript_events
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>(),
        ["reply finished on the old selection"]
    );
}

#[test]
fn a_clear_mid_turn_gives_the_next_turn_no_history() {
    let context = next_turn_context(ContextPolicy::Clear);
    assert!(context.entries.is_empty());
    assert!(context.transcript_events.is_empty());
}
