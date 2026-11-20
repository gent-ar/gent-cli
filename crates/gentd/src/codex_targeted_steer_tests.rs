use super::*;
use gent_drivers::codex_session::CodexSteerOutcome;
#[path = "codex_stop_withdraw_tests.rs"]
mod stop_withdraw;

#[test]
fn a_prompt_queued_before_the_codex_turn_starts_waits_for_the_turn() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = setup_conversation(&ledger);
    save(&ledger, &conversation, "active", "first");
    let original = ledger.list_run_turns("run-a").unwrap()[0].turn_id.clone();
    let queued = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("queued".into()),
            receipt_id: ReceiptId("queued-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            text: "later".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &queued);
    let runner = Runner::default();
    runner.state.lock().unwrap().turn_pending = true;
    let mut host = host(&ledger, runner.clone());
    host.tick().unwrap();
    host.tick().unwrap();
    host.tick().unwrap();
    let phase = |turn: &str| ledger.find_turn(turn).unwrap().unwrap().phase;
    assert!(runner.state.lock().unwrap().steers.is_empty());
    assert_eq!(phase(&queued.message.turn_id), DurableTurnPhase::Active);

    runner.state.lock().unwrap().turn_pending = false;
    runner
        .state
        .lock()
        .unwrap()
        .effects
        .push_back(vec![CodexRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "native-thread".into(),
            },
        )]);
    host.tick().unwrap();
    assert_eq!(runner.state.lock().unwrap().steers.len(), 1);
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Steer(CodexSteerOutcome::Consumed {
            message_id: queued.message.message_id.clone(),
        }),
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )),
    ]);
    host.tick().unwrap();
    assert_eq!(phase(&original), DurableTurnPhase::Completed);
    assert_eq!(phase(&queued.message.turn_id), DurableTurnPhase::Completed);
    let facts = ledger
        .read_conversation_activity_page(&conversation.0, "run-a", 0, 50)
        .unwrap()
        .facts;
    assert!(facts.iter().any(|fact| matches!(fact, ConversationActivityFact::PromptSteered { scope, message_id, .. } if scope.turn_id == original && message_id == &queued.message.message_id)));
    assert!(
        !facts
            .iter()
            .any(|fact| matches!(fact, ConversationActivityFact::PromptSteerRejected { .. }))
    );
}

#[test]
fn a_joined_prompt_is_steered_when_consumed_and_otherwise_runs_as_the_next_turn() {
    for ordering in ["before", "after", "absent", "unconfirmed", "rejected"] {
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = setup_conversation(&ledger);
        save(&ledger, &conversation, "active", "first");
        let original = ledger.list_run_turns("run-a").unwrap()[0].turn_id.clone();
        let queued = ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId("queued".into()),
                receipt_id: ReceiptId("queued-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                disposition: AgentChatPromptDisposition::Queue,
                text: "later".into(),
                attachment_ids: vec![],
                tool_source_ids: vec![],
            })
            .unwrap();
        crate::readiness_test_support::release(&ledger, &queued);
        let runner = Runner::default();
        let mut host = host(&ledger, runner.clone());
        host.tick().unwrap();
        runner
            .state
            .lock()
            .unwrap()
            .effects
            .push_back(vec![CodexRunnerEffect::Fact(
                PublicWireFact::SessionStarted {
                    provider_session_id: "native-thread".into(),
                },
            )]);
        host.tick().unwrap();
        assert_eq!(runner.state.lock().unwrap().steers.len(), 1);
        let terminal = CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        ));
        let consumed = CodexRunnerEffect::Steer(CodexSteerOutcome::Consumed {
            message_id: queued.message.message_id.clone(),
        });
        let effects = match ordering {
            "before" => vec![consumed, terminal],
            "after" => vec![terminal, consumed],
            "unconfirmed" => vec![
                CodexRunnerEffect::Steer(CodexSteerOutcome::Unconfirmed {
                    message_id: queued.message.message_id.clone(),
                }),
                terminal,
            ],
            "rejected" => vec![
                terminal,
                CodexRunnerEffect::Steer(CodexSteerOutcome::Rejected {
                    message_id: queued.message.message_id.clone(),
                }),
            ],
            _ => vec![terminal],
        };
        runner.state.lock().unwrap().effects.push_back(effects);
        host.tick().unwrap();
        assert_eq!(
            ledger.find_turn(&original).unwrap().unwrap().phase,
            DurableTurnPhase::Completed
        );
        let facts = ledger
            .read_conversation_activity_page(&conversation.0, "run-a", 0, 50)
            .unwrap()
            .facts;
        let joined = matches!(ordering, "before" | "after");
        let released = facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptReleased {message_id,..} if message_id==&queued.message.message_id));
        assert_eq!(released, !joined, "{ordering}: {facts:?}");
        assert_eq!(
            joined,
            facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteered {scope,message_id,receipt_id,..} if scope.turn_id==original && message_id==&queued.message.message_id && receipt_id=="queued-receipt")),
            "{ordering}"
        );
        assert!(
            !facts
                .iter()
                .any(|fact| matches!(fact, ConversationActivityFact::PromptSteerRejected { .. })),
            "{ordering}"
        );
        let phase = ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase;
        assert_eq!(
            phase,
            if joined {
                DurableTurnPhase::Completed
            } else {
                DurableTurnPhase::Active
            },
            "{ordering}"
        );
        assert_eq!(
            runner.state.lock().unwrap().submitted.len(),
            usize::from(!joined),
            "{ordering}"
        );
        assert_eq!(runner.state.lock().unwrap().starts, 1);
    }
}
