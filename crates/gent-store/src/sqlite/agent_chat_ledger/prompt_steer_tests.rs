use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatQueuedPromptLedger,
    AgentChatWorkspaceLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};

use crate::SqliteLedger;

const EPOCH: HostEpoch = HostEpoch(1);
const COORDINATOR: &str = "gentd-1";

fn queued(ledger: &SqliteLedger, request_id: &str) -> gent_types::AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt_with_origin(
            &AgentChatPromptCreate {
                request_id: AgentChatRequestId(request_id.into()),
                receipt_id: ReceiptId(request_id.into()),
                host_epoch: EPOCH,
                conversation_id: AgentChatConversationId("conversation-1".into()),
                disposition: AgentChatPromptDisposition::Queue,
                text: request_id.into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
            &AgentChatPromptOrigin::User {
                client_request_id: None,
            },
        )
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(&saved.message.message_id, &saved.run_id, EPOCH)
        .unwrap();
    saved
}

fn root_turn(ledger: &SqliteLedger) -> gent_types::AgentChatPromptSaved {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: EPOCH,
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "haiku".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let root = ledger
        .save_agent_chat_prompt_with_origin(
            &AgentChatPromptCreate {
                request_id: AgentChatRequestId("root".into()),
                receipt_id: ReceiptId("root".into()),
                host_epoch: EPOCH,
                conversation_id: AgentChatConversationId("conversation-1".into()),
                disposition: AgentChatPromptDisposition::Send,
                text: "Go".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            },
            &AgentChatPromptOrigin::User {
                client_request_id: None,
            },
        )
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(&root.message.message_id, &root.run_id, EPOCH)
        .unwrap();
    ledger
        .claim_agent_chat_prompt_dispatch(COORDINATOR, EPOCH, AgentChatProvider::Claude)
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&root.message.message_id, COORDINATOR, EPOCH)
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&root.message.message_id, COORDINATOR, EPOCH)
        .unwrap();
    root
}

fn claim(ledger: &SqliteLedger, root: &gent_types::AgentChatPromptSaved) -> Option<String> {
    ledger
        .claim_steered_agent_chat_prompt_for_turn(
            COORDINATOR,
            EPOCH,
            &root.run_id,
            &root.message.turn_id,
        )
        .unwrap()
        .map(|prompt| prompt.message.message_id)
}

#[test]
fn a_prompt_the_provider_returns_unconsumed_rejoins_the_turn_ahead_of_younger_prompts() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let root = root_turn(&ledger);
    let returned = queued(&ledger, "returned");
    let younger = queued(&ledger, "younger");

    assert_eq!(
        claim(&ledger, &root),
        Some(returned.message.message_id.clone())
    );
    ledger
        .begin_agent_chat_prompt_launch(&returned.message.message_id, COORDINATOR, EPOCH)
        .unwrap();
    ledger
        .release_agent_chat_prompt_unstarted_launch(
            &returned.message.message_id,
            COORDINATOR,
            EPOCH,
        )
        .unwrap();

    assert_eq!(
        claim(&ledger, &root),
        Some(returned.message.message_id.clone())
    );
    ledger
        .begin_agent_chat_prompt_launch(&returned.message.message_id, COORDINATOR, EPOCH)
        .unwrap();
    ledger
        .deliver_steered_agent_chat_prompt(
            &returned.message.message_id,
            COORDINATOR,
            EPOCH,
            &root.message.turn_id,
        )
        .unwrap();
    assert_eq!(
        claim(&ledger, &root),
        Some(younger.message.message_id.clone())
    );
}

#[test]
fn send_now_stops_further_prompts_from_joining_the_turn_it_interrupts() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let root = root_turn(&ledger);
    let pressed = queued(&ledger, "pressed");
    let (_, run, turn) = ledger
        .steer_queued_agent_chat_prompt_for_turn(
            &ReceiptId("send-now".into()),
            EPOCH,
            &AgentChatConversationId("conversation-1".into()),
            &pressed.message.message_id,
            &root.run_id,
            &root.message.turn_id,
        )
        .map(|(receipt, run)| (receipt, run, root.message.turn_id.clone()))
        .unwrap();
    assert_eq!(run, root.run_id);
    assert_eq!(turn, root.message.turn_id);
    queued(&ledger, "later");
    assert_eq!(claim(&ledger, &root), None);
}
