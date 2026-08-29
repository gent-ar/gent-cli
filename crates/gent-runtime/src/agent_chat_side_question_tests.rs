use super::{
    AgentChatSideQuestionAskResult, AgentChatSideQuestionAuthority, AgentChatSideQuestionService,
    bounded_excerpt,
};
use gent_ports::{AgentChatWorkspaceLedger, ConversationSummaryRunner, PortError};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    AgentChatSideQuestion, AgentChatSideQuestionAsked, AgentChatSideQuestionCancel,
    AgentChatSideQuestionStatus, HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    ReceiptId, WorkspaceRecord,
};

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "claude-sonnet".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

fn conversation(ledger: &SqliteLedger, conversation_id: &str) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId(format!("{conversation_id}-create-receipt")),
                idempotency_key: format!("{conversation_id}-create-key"),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId(conversation_id.into()),
                run_id: AgentChatRunId(format!("{conversation_id}-run")),
                selection: selection(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace-1".into(),
            },
        )
        .unwrap();
}

fn approved(ledger: SqliteLedger) -> AgentChatSideQuestionService<SqliteLedger> {
    AgentChatSideQuestionService::new(ledger, AgentChatSideQuestionAuthority::Approved)
}

fn ask(conversation_id: &str, request_id: &str, question: &str) -> AgentChatSideQuestion {
    AgentChatSideQuestion {
        request_id: AgentChatRequestId(request_id.into()),
        receipt_id: ReceiptId(format!("{request_id}-receipt")),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId(conversation_id.into()),
        question: question.into(),
        created_at_unix_ms: 1_000,
    }
}

fn asked(
    service: &AgentChatSideQuestionService<SqliteLedger>,
    ask: &AgentChatSideQuestion,
) -> AgentChatSideQuestionAsked {
    let AgentChatSideQuestionAskResult::Asked(asked) = service.ask(ask).unwrap() else {
        unreachable!()
    };
    asked
}

fn message(
    kind: NormalizedTranscriptKind,
    text: &str,
    cursor: u64,
    is_partial: bool,
) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn-1".into(),
        run_id: "run-1".into(),
        kind,
        text: text.into(),
        is_partial,
    }
}

struct FakeRunner {
    answer: Option<String>,
}

impl ConversationSummaryRunner for FakeRunner {
    fn run_summary(
        &self,
        _provider: &str,
        _model_version: &str,
        _prompt: &str,
    ) -> Result<String, PortError> {
        self.answer
            .clone()
            .ok_or_else(|| PortError::Unavailable("fake runner failure".into()))
    }
}

#[test]
fn bounded_excerpt_keeps_only_the_newest_messages_in_order() {
    let events: Vec<NormalizedTranscriptEvent> = (0..20)
        .map(|index| {
            message(
                NormalizedTranscriptKind::UserMessage,
                &format!("m{index}"),
                index,
                false,
            )
        })
        .collect();
    let excerpt = bounded_excerpt(&events);
    assert!(excerpt.contains("m19"));
    assert!(excerpt.contains("m12"));
    assert!(!excerpt.contains("m11"));
    assert!(excerpt.find("m12").unwrap() < excerpt.find("m19").unwrap());
}

#[test]
fn bounded_excerpt_excludes_partial_and_non_message_events() {
    let events = vec![
        message(
            NormalizedTranscriptKind::ToolActivity,
            "tool noise",
            0,
            false,
        ),
        message(NormalizedTranscriptKind::UserMessage, "streaming", 1, true),
        message(
            NormalizedTranscriptKind::AssistantMessage,
            "final answer",
            2,
            false,
        ),
    ];
    let excerpt = bounded_excerpt(&events);
    assert_eq!(excerpt, "Assistant: final answer");
}

#[test]
fn bounded_excerpt_clips_to_its_byte_bound_keeping_the_tail() {
    let long = "x".repeat(20_000);
    let events = vec![message(
        NormalizedTranscriptKind::UserMessage,
        &long,
        0,
        false,
    )];
    let excerpt = bounded_excerpt(&events);
    assert!(excerpt.len() <= 12_000);
    assert!(long.ends_with(&excerpt));
}

#[test]
fn observer_authority_denies_every_ask() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service =
        AgentChatSideQuestionService::new(ledger, AgentChatSideQuestionAuthority::Observer);
    let result = service
        .ask(&ask("conversation-1", "request-1", "why?"))
        .unwrap();
    assert_eq!(result, AgentChatSideQuestionAskResult::DeniedObserver);
}

#[test]
fn asking_an_unknown_conversation_is_rejected() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let service = approved(ledger);
    let result = service.ask(&ask("missing-conversation", "request-1", "why?"));
    assert!(result.is_err());
}

#[test]
fn asking_enforces_the_per_conversation_live_bound() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = approved(ledger);
    for index in 0..3 {
        asked(
            &service,
            &ask("conversation-1", &format!("request-{index}"), "why?"),
        );
    }
    let result = service.ask(&ask("conversation-1", "request-overflow", "why?"));
    assert!(result.is_err());
}

#[test]
fn asking_the_same_request_twice_returns_the_same_side_question() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = approved(ledger);
    let first = asked(&service, &ask("conversation-1", "request-1", "why?"));
    let second = asked(&service, &ask("conversation-1", "request-1", "why?"));
    assert_eq!(
        first.record.side_question_id,
        second.record.side_question_id
    );
}

#[test]
fn run_and_complete_settles_a_successful_run_as_answered() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = approved(ledger);
    let record = asked(&service, &ask("conversation-1", "request-1", "why?"));
    let settled = service
        .run_and_complete(
            &record.record.side_question_id,
            "conversation-1",
            "why?",
            |_provider, _workspace_path| {
                Ok(Box::new(FakeRunner {
                    answer: Some("because".into()),
                }) as Box<dyn ConversationSummaryRunner>)
            },
        )
        .unwrap();
    assert_eq!(settled.status, AgentChatSideQuestionStatus::Answered);
    assert_eq!(settled.answer.as_deref(), Some("because"));
}

#[test]
fn run_and_complete_settles_a_failed_run_as_failed_not_an_error() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = approved(ledger);
    let record = asked(&service, &ask("conversation-1", "request-1", "why?"));
    let settled = service
        .run_and_complete(
            &record.record.side_question_id,
            "conversation-1",
            "why?",
            |_provider, _workspace_path| {
                Ok(Box::new(FakeRunner { answer: None }) as Box<dyn ConversationSummaryRunner>)
            },
        )
        .unwrap();
    assert_eq!(settled.status, AgentChatSideQuestionStatus::Failed);
    assert!(settled.failure_reason.is_some());
}

#[test]
fn cancel_transitions_a_pending_side_question_and_completion_no_longer_overwrites_it() {
    let ledger = SqliteLedger::in_memory().unwrap();
    conversation(&ledger, "conversation-1");
    let service = approved(ledger);
    let record = asked(&service, &ask("conversation-1", "request-1", "why?"));
    let cancelled = service
        .cancel(&AgentChatSideQuestionCancel {
            request_id: AgentChatRequestId("cancel-request".into()),
            receipt_id: ReceiptId("cancel-receipt".into()),
            host_epoch: HostEpoch(1),
            side_question_id: record.record.side_question_id.clone(),
        })
        .unwrap();
    let super::AgentChatSideQuestionCancelResult::Cancelled(cancelled) = cancelled else {
        unreachable!()
    };
    assert_eq!(
        cancelled.record.status,
        AgentChatSideQuestionStatus::Cancelled
    );
    let settled = service
        .run_and_complete(
            &record.record.side_question_id,
            "conversation-1",
            "why?",
            |_provider, _workspace_path| {
                Ok(Box::new(FakeRunner {
                    answer: Some("too late".into()),
                }) as Box<dyn ConversationSummaryRunner>)
            },
        )
        .unwrap();
    assert_eq!(settled.status, AgentChatSideQuestionStatus::Cancelled);
    assert!(settled.answer.is_none());
}
