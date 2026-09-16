use gent_ports::{
    AgentChatProjectionLedger, AgentChatWorkspaceLedger, ConversationActivityLedger,
    ConversationLinkLedger, LedgerError,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, ConversationActivityFact,
    ConversationActivityScope, ConversationLink, ConversationMessageDelivery,
    ConversationWaitTarget, DurableTurnPhase, HostEpoch, ReceiptId, WorkspaceRecord,
};

use super::SqliteLedger;

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn conversation(ledger: &SqliteLedger, conversation_id: &str, run_id: &str) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("receipt-{conversation_id}")),
                idempotency_key: format!("key-{conversation_id}"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation_id.into()),
                run_id: AgentChatRunId(run_id.into()),
                selection: selection(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
}

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "parent", "run-parent");
    conversation(&ledger, "child", "run-child");
    ledger
}

fn activity(
    ledger: &SqliteLedger,
    conversation_id: &str,
    run_id: &str,
) -> gent_types::ConversationActivityPage {
    ledger
        .read_conversation_activity_page(conversation_id, run_id, 0, 16)
        .unwrap()
}

fn scope(conversation_id: &str, run_id: &str) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: conversation_id.into(),
        run_id: run_id.into(),
        turn_id: format!("links:{conversation_id}"),
        host_epoch: HostEpoch(1),
        cursor: 0,
    }
}

fn link() -> ConversationLink {
    ConversationLink {
        parent_conversation_id: "parent".into(),
        child_conversation_id: "child".into(),
        label: "build the parser".into(),
        created_run_id: "run-parent".into(),
        created_turn_id: "links:parent".into(),
        created_at_unix_seconds: 10,
    }
}

fn created_facts() -> Vec<ConversationActivityFact> {
    vec![
        ConversationActivityFact::ConversationCreated {
            scope: scope("parent", "run-parent"),
            child_conversation_id: "child".into(),
            label: "build the parser".into(),
            workspace_path: "/workspace".into(),
            selection: selection(),
            origin_tool_use_id: Some("tool-1".into()),
        },
        ConversationActivityFact::CreatedByConversation {
            scope: scope("child", "run-child"),
            parent_conversation_id: "parent".into(),
            parent_run_id: "run-parent".into(),
            label: "the planner".into(),
        },
    ]
}

#[test]
fn a_recorded_link_is_readable_from_both_ends_and_appends_one_fact_to_each_conversation() {
    let ledger = ledger();
    ledger
        .record_conversation_link(&link(), &created_facts(), "request-1")
        .unwrap();

    assert_eq!(
        ledger.read_conversation_parent("child").unwrap(),
        Some(link())
    );
    assert_eq!(
        ledger.read_conversation_children("parent").unwrap(),
        vec![link()]
    );
    assert!(ledger.read_conversation_parent("parent").unwrap().is_none());
    assert!(
        ledger
            .read_conversation_children("child")
            .unwrap()
            .is_empty()
    );

    let parent_facts = activity(&ledger, "parent", "run-parent");
    assert!(matches!(
        parent_facts.facts.as_slice(),
        [ConversationActivityFact::ConversationCreated { child_conversation_id, label, .. }]
            if child_conversation_id == "child" && label == "build the parser"
    ));
    let child_facts = activity(&ledger, "child", "run-child");
    assert!(matches!(
        child_facts.facts.as_slice(),
        [ConversationActivityFact::CreatedByConversation { parent_conversation_id, label, .. }]
            if parent_conversation_id == "parent" && label == "the planner"
    ));
}

#[test]
fn a_repeated_request_never_duplicates_the_link_or_its_facts() {
    let ledger = ledger();
    ledger
        .record_conversation_link(&link(), &created_facts(), "request-1")
        .unwrap();
    ledger
        .record_conversation_link(&link(), &created_facts(), "request-1")
        .unwrap();

    assert_eq!(
        ledger.read_conversation_children("parent").unwrap().len(),
        1
    );
    assert_eq!(activity(&ledger, "parent", "run-parent").facts.len(), 1);
}

#[test]
fn a_child_may_never_be_reparented() {
    let ledger = ledger();
    conversation(&ledger, "other", "run-other");
    ledger
        .record_conversation_link(&link(), &created_facts(), "request-1")
        .unwrap();
    let mut stolen = link();
    stolen.parent_conversation_id = "other".into();
    assert!(matches!(
        ledger.record_conversation_link(&stolen, &[], "request-2"),
        Err(LedgerError::Invariant(_))
    ));
    assert_eq!(
        ledger
            .read_conversation_parent("child")
            .unwrap()
            .map(|link| link.parent_conversation_id),
        Some("parent".into())
    );
}

#[test]
fn a_link_without_distinct_identities_or_a_label_is_refused() {
    let ledger = ledger();
    let mut circular = link();
    circular.child_conversation_id = "parent".into();
    assert!(matches!(
        ledger.record_conversation_link(&circular, &[], "request-1"),
        Err(LedgerError::Invariant(_))
    ));
    let mut unlabeled = link();
    unlabeled.label = "   ".into();
    assert!(matches!(
        ledger.record_conversation_link(&unlabeled, &[], "request-2"),
        Err(LedgerError::Invariant(_))
    ));
}

#[test]
fn orchestration_facts_carry_target_labels_and_reach_the_projection() {
    let ledger = ledger();
    ledger
        .append_conversation_orchestration_fact(
            &ConversationActivityFact::ConversationMessageSent {
                scope: scope("parent", "run-parent"),
                target_conversation_id: "child".into(),
                target_label: "build the parser".into(),
                delivery: ConversationMessageDelivery::Steered,
                preview: "keep going".into(),
            },
            "send:request-1",
        )
        .unwrap();
    ledger
        .append_conversation_orchestration_fact(
            &ConversationActivityFact::ConversationWaitSettled {
                scope: scope("parent", "run-parent"),
                targets: vec![ConversationWaitTarget {
                    conversation_id: "child".into(),
                    label: "build the parser".into(),
                    phase: Some(DurableTurnPhase::Completed),
                }],
                timed_out: false,
            },
            "wait:request-1",
        )
        .unwrap();

    let facts = activity(&ledger, "parent", "run-parent");
    assert!(matches!(
        &facts.facts[0],
        ConversationActivityFact::ConversationMessageSent { target_label, delivery, .. }
            if target_label == "build the parser"
                && *delivery == ConversationMessageDelivery::Steered
    ));
    assert!(matches!(
        &facts.facts[1],
        ConversationActivityFact::ConversationWaitSettled { targets, timed_out, .. }
            if targets[0].label == "build the parser" && !timed_out
    ));
    assert!(facts.facts[0].scope().cursor < facts.facts[1].scope().cursor);
    assert_eq!(
        ledger
            .agent_chat_projection_page(&AgentChatConversationId("parent".into()), 0, 16)
            .unwrap()
            .events
            .len(),
        2
    );
}
