use gent_ports::{ContextCompactionLedger, TranscriptLedger};
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, AgentChatPromptAuthority, AgentChatPromptRequest,
    AgentChatPromptResult, AgentChatPromptService, AgentChatRunContextService,
    AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchRequest,
    AgentChatSelectionSwitchResult, AgentChatSelectionSwitchService, ContextCompactionBudget,
    ContextCompactionDecision, ConversationContextArtifactService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptDisposition,
    AgentChatProvider, AgentChatRequestId, AgentChatRunContext, AgentChatRunId, AgentChatSelection,
    ContextCompactionFailure, ContextCompactionPlan, ContextCompactionTrigger, ContextPolicy,
    ContextSourceRole, ConversationMessage, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

const BUDGET: ContextCompactionBudget = ContextCompactionBudget {
    tail_bytes: 1_000_000,
    tail_entries: 64,
    tail_transcript_items: 100,
    source_bytes: 1_000_000,
    item_bytes: 4_096,
};

struct Chat {
    ledger: SqliteLedger,
    conversation: AgentChatConversationId,
    run: AgentChatRunId,
    messages: Vec<ConversationMessage>,
}

impl Chat {
    fn new() -> Self {
        let ledger = SqliteLedger::in_memory().unwrap();
        let AgentChatConversationResult::Created(created) = AgentChatConversationService::new(
            ledger.clone(),
            AgentChatConversationAuthority::Approved,
        )
        .create(&AgentChatConversationRequest {
            request_id: AgentChatRequestId("create".into()),
            receipt_id: ReceiptId("create".into()),
            host_epoch: HostEpoch(1),
            selection: selection(),
            workspace: WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        })
        .unwrap() else {
            panic!("conversation must be created")
        };
        Self {
            ledger,
            conversation: created.conversation_id,
            run: created.run_id,
            messages: Vec::new(),
        }
    }

    fn say(&mut self, text: &str) -> ConversationMessage {
        let index = self.messages.len();
        let AgentChatPromptResult::Saved(saved) =
            AgentChatPromptService::new(self.ledger.clone(), AgentChatPromptAuthority::Approved)
                .submit(&AgentChatPromptRequest {
                    request_id: AgentChatRequestId(format!("prompt-{index}")),
                    receipt_id: ReceiptId(format!("prompt-{index}")),
                    host_epoch: HostEpoch(1),
                    conversation_id: self.conversation.clone(),
                    disposition: AgentChatPromptDisposition::Queue,
                    text: text.into(),
                    attachment_ids: vec![],
                    tool_source_ids: vec![],
                })
                .unwrap()
        else {
            panic!("prompt must be saved")
        };
        self.messages.push(saved.message.clone());
        saved.message
    }

    fn reply(&self, message: &ConversationMessage, text: &str) {
        self.ledger
            .append_normalized_transcript(
                &self.conversation,
                &NormalizedTranscriptAppend {
                    event_id: format!("reply:{}", message.message_id),
                    turn_id: message.turn_id.clone(),
                    run_id: message.run_id.clone(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: text.into(),
                    is_partial: false,
                },
            )
            .unwrap();
    }

    fn context(&self) -> AgentChatRunContext {
        AgentChatRunContextService::new(self.ledger.clone())
            .resolve(&self.conversation, &self.run)
            .unwrap()
    }

    fn artifacts(&self) -> ConversationContextArtifactService<SqliteLedger> {
        ConversationContextArtifactService::new(self.ledger.clone())
    }

    fn plan(
        &self,
        message: &ConversationMessage,
        trigger: ContextCompactionTrigger,
        budget: ContextCompactionBudget,
    ) -> ContextCompactionDecision {
        self.artifacts()
            .plan_run_compaction(
                &self.context(),
                (&message.message_id, &message.turn_id),
                trigger,
                budget,
            )
            .unwrap()
    }

    fn compact(&self, message: &ConversationMessage, summary: &str) -> ContextCompactionPlan {
        let ContextCompactionDecision::Plan(plan) =
            self.plan(message, ContextCompactionTrigger::Command, BUDGET)
        else {
            panic!("a command always plans")
        };
        self.ledger
            .record_context_compaction(&plan.compacted(summary.into(), 7), HostEpoch(1))
            .unwrap();
        *plan
    }

    fn switch(&mut self, id: &str, policy: ContextPolicy) {
        let AgentChatSelectionSwitchResult::Switched(result) =
            AgentChatSelectionSwitchService::new(
                self.ledger.clone(),
                AgentChatSelectionSwitchAuthority::Approved,
            )
            .switch(&AgentChatSelectionSwitchRequest {
                request_id: AgentChatRequestId(id.into()),
                receipt_id: ReceiptId(id.into()),
                host_epoch: HostEpoch(1),
                conversation_id: self.conversation.clone(),
                parent_run_id: self.run.clone(),
                selection: AgentChatSelection {
                    effort: AgentChatEffort::High,
                    ..selection()
                },
                context_policy: policy,
            })
            .unwrap()
        else {
            panic!("selection must switch")
        };
        self.run = result.run_id;
    }
}

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Agent,
    }
}

fn texts(context: &gent_types::FrozenConversationContext) -> Vec<&str> {
    context
        .entries
        .iter()
        .map(|entry| entry.text.as_str())
        .collect()
}

#[test]
fn a_summary_replaces_the_turns_it_covers_and_keeps_every_later_turn() {
    let mut chat = Chat::new();
    let first = chat.say("Remember LARK-7");
    chat.reply(&first, "Noted LARK-7");
    let command = chat.say("/compact");
    let later = chat.say("later question");
    let current = chat.say("current question");
    let plan = chat.compact(&command, "The user planted LARK-7.");
    assert_eq!(plan.covers_through_ordinal, 2);
    assert_eq!(
        plan.items
            .iter()
            .map(|item| (item.role, item.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (ContextSourceRole::User, "Remember LARK-7"),
            (ContextSourceRole::Assistant, "Noted LARK-7")
        ]
    );

    let summarized = chat
        .artifacts()
        .project_run_summarized(&chat.context(), &current.message_id)
        .unwrap()
        .unwrap();
    let summary = summarized.summary.as_ref().unwrap();
    assert_eq!(summary.text, "The user planted LARK-7.");
    assert_eq!(summary.covers_through_ordinal, 2);
    assert_eq!(texts(&summarized), ["later question"]);
    assert!(summarized.transcript_events.is_empty());
    assert!(!summarized.earlier_history_omitted);
    assert_eq!(later.text, "later question");

    let complete = chat
        .artifacts()
        .project_run_before_message(&chat.context(), &current.message_id)
        .unwrap();
    assert_eq!(
        texts(&complete),
        ["Remember LARK-7", "/compact", "later question"]
    );
    assert!(complete.summary.is_none());
}

#[test]
fn a_cleared_run_ignores_the_summary_and_a_preserved_switch_reuses_it() {
    let mut chat = Chat::new();
    chat.say("Remember LARK-7");
    let command = chat.say("/compact");
    chat.compact(&command, "The user planted LARK-7.");

    chat.switch("preserve", ContextPolicy::Preserve);
    let preserved = chat.say("after preserve");
    assert!(
        chat.artifacts()
            .project_run_summarized(&chat.context(), &preserved.message_id)
            .unwrap()
            .is_some()
    );

    chat.switch("clear", ContextPolicy::Clear);
    let cleared = chat.say("after clear");
    assert!(
        chat.artifacts()
            .project_run_summarized(&chat.context(), &cleared.message_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_fact_whose_coverage_no_longer_matches_the_run_view_is_never_used() {
    let mut chat = Chat::new();
    chat.say("Remember LARK-7");
    let command = chat.say("/compact");
    let ContextCompactionDecision::Plan(mut plan) =
        chat.plan(&command, ContextCompactionTrigger::Command, BUDGET)
    else {
        panic!("a command always plans")
    };
    plan.covered_digest_sha256 = "f".repeat(64);
    chat.ledger
        .record_context_compaction(&plan.compacted("forged".into(), 1), HostEpoch(1))
        .unwrap();
    let current = chat.say("current");
    assert!(
        chat.artifacts()
            .project_run_summarized(&chat.context(), &current.message_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_budget_plan_keeps_the_newest_turns_within_the_tail_and_covers_the_rest() {
    let mut chat = Chat::new();
    let text = |index| format!("{index}{}", "x".repeat(99));
    for index in 1..=5 {
        chat.say(&text(index));
    }
    let current = chat.say("current");
    let budget = ContextCompactionBudget {
        tail_bytes: 800,
        ..BUDGET
    };
    let ContextCompactionDecision::Plan(plan) =
        chat.plan(&current, ContextCompactionTrigger::Budget, budget)
    else {
        panic!("history beyond the tail must plan")
    };
    assert_eq!(plan.covers_through_ordinal, 3);
    assert_eq!(plan.turn_id, current.turn_id);
    assert_eq!(
        plan.items
            .iter()
            .map(|item| item.text.clone())
            .collect::<Vec<_>>(),
        [text(1), text(2), text(3)]
    );
    assert_eq!(
        chat.plan(&current, ContextCompactionTrigger::Budget, BUDGET),
        ContextCompactionDecision::NothingToCover
    );
    let entries = ContextCompactionBudget {
        tail_entries: 1,
        ..BUDGET
    };
    let ContextCompactionDecision::Plan(plan) =
        chat.plan(&current, ContextCompactionTrigger::Budget, entries)
    else {
        panic!("history beyond the tail entry bound must plan")
    };
    assert_eq!(plan.covers_through_ordinal, 4);
}

#[test]
fn a_rolling_compaction_carries_the_previous_summary_and_only_new_source() {
    let mut chat = Chat::new();
    chat.say("Remember LARK-7");
    let first = chat.say("/compact");
    chat.compact(&first, "The user planted LARK-7.");
    let newer = chat.say("Remember OWL-3");
    chat.reply(&newer, "Noted OWL-3");
    let second = chat.say("/compact");
    let ContextCompactionDecision::Plan(plan) =
        chat.plan(&second, ContextCompactionTrigger::Command, BUDGET)
    else {
        panic!("a command always plans")
    };
    assert_eq!(
        plan.previous_summary.as_deref(),
        Some("The user planted LARK-7.")
    );
    assert_eq!(plan.covers_through_ordinal, 4);
    assert_eq!(
        plan.items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        ["Remember OWL-3", "Noted OWL-3"]
    );
    let small = ContextCompactionBudget {
        source_bytes: "Noted OWL-3".len() + 64,
        ..BUDGET
    };
    let ContextCompactionDecision::Plan(bounded) =
        chat.plan(&second, ContextCompactionTrigger::Command, small)
    else {
        panic!("a command always plans")
    };
    assert_eq!(bounded.omitted_source_items, 1);
    assert_eq!(bounded.items.len(), 1);
}

#[test]
fn a_failed_budget_attempt_backs_off_until_enough_new_history_exists() {
    let mut chat = Chat::new();
    for index in 1..=3 {
        chat.say(&format!("message {index}"));
    }
    let current = chat.say("current");
    let tight = ContextCompactionBudget {
        tail_entries: 1,
        ..BUDGET
    };
    let ContextCompactionDecision::Plan(plan) =
        chat.plan(&current, ContextCompactionTrigger::Budget, tight)
    else {
        panic!("history beyond the tail must plan")
    };
    chat.ledger
        .record_context_compaction(
            &plan.failed(ContextCompactionFailure::OutputLimit),
            HostEpoch(1),
        )
        .unwrap();
    assert_eq!(
        chat.plan(&current, ContextCompactionTrigger::Budget, tight),
        ContextCompactionDecision::BackingOff
    );
    assert!(matches!(
        chat.plan(&current, ContextCompactionTrigger::Command, tight),
        ContextCompactionDecision::Plan(_)
    ));
    for index in 0..8 {
        chat.say(&format!("more {index}"));
    }
    let later = chat.say("later");
    assert!(matches!(
        chat.plan(&later, ContextCompactionTrigger::Budget, tight),
        ContextCompactionDecision::Plan(_)
    ));
}
