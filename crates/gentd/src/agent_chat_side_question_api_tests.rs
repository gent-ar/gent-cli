use crate::agent_chat_side_question_api::exchange;
use gent_protocol::AgentChatSideQuestionFrame;
use gent_ports::AgentChatWorkspaceLedger;
use gent_runtime::AgentChatSideQuestionService;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};

fn conversation(ledger: &SqliteLedger, conversation_id: &str) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("{conversation_id}-create-receipt")),
                idempotency_key: format!("{conversation_id}-create-key"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation_id.into()),
                run_id: AgentChatRunId(format!("{conversation_id}-run")),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-sonnet".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace-1".into(),
            },
        )
        .unwrap();
}

#[test]
fn asking_a_side_question_returns_it_pending() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = AgentChatSideQuestionService::new(ledger, gent_runtime::AgentChatSideQuestionAuthority::Approved);
    let response = exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "request-1".into(),
            receipt_id: "receipt-1".into(),
            conversation_id: "conversation-1".into(),
            question: "why did this fail?".into(),
        },
    )
    .unwrap();
    let AgentChatSideQuestionFrame::Asked { record, .. } = response else {
        unreachable!()
    };
    assert_eq!(record.status, gent_types::AgentChatSideQuestionStatus::Pending);
    assert_eq!(record.conversation_id.0, "conversation-1");
}

#[test]
fn listing_side_questions_reads_every_asked_question() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = AgentChatSideQuestionService::new(ledger, gent_runtime::AgentChatSideQuestionAuthority::Approved);
    exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "request-1".into(),
            receipt_id: "receipt-1".into(),
            conversation_id: "conversation-1".into(),
            question: "why?".into(),
        },
    )
    .unwrap();
    let response = exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::ListSideQuestions {
            request_id: "request-2".into(),
            conversation_id: "conversation-1".into(),
        },
    )
    .unwrap();
    let AgentChatSideQuestionFrame::SideQuestions { side_questions, .. } = response else {
        unreachable!()
    };
    assert_eq!(side_questions.len(), 1);
}

#[test]
fn cancelling_a_side_question_marks_it_cancelled() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = AgentChatSideQuestionService::new(ledger, gent_runtime::AgentChatSideQuestionAuthority::Approved);
    let asked = exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "request-1".into(),
            receipt_id: "receipt-1".into(),
            conversation_id: "conversation-1".into(),
            question: "why?".into(),
        },
    )
    .unwrap();
    let AgentChatSideQuestionFrame::Asked { record, .. } = asked else {
        unreachable!()
    };
    let cancelled = exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::CancelSideQuestion {
            request_id: "request-2".into(),
            receipt_id: "receipt-2".into(),
            side_question_id: record.side_question_id,
        },
    )
    .unwrap();
    let AgentChatSideQuestionFrame::Cancelled { record, .. } = cancelled else {
        unreachable!()
    };
    assert_eq!(record.status, gent_types::AgentChatSideQuestionStatus::Cancelled);
}

#[test]
fn server_only_response_frames_are_rejected_as_requests() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let service = AgentChatSideQuestionService::new(ledger, gent_runtime::AgentChatSideQuestionAuthority::Approved);
    let result = exchange(
        &service,
        HostEpoch(1),
        AgentChatSideQuestionFrame::SideQuestions {
            request_id: "request-1".into(),
            side_questions: Vec::new(),
        },
    );
    assert!(result.is_err());
}
