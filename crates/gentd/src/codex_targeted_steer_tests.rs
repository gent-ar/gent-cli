use super::*;
use gent_drivers::codex_session::CodexSteerOutcome;
use gent_ports::AgentChatQueuedPromptLedger;

#[test]
fn targeted_steer_consumption_is_bound_before_terminal_or_becomes_unprovable() {
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
        ledger
            .steer_queued_agent_chat_prompt_for_turn(
                &ReceiptId("steer".into()),
                HostEpoch(1),
                &conversation,
                &queued.message.message_id,
                &AgentChatRunId("run-a".into()),
                &original,
            )
            .unwrap();
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
        assert!(!facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptReleased {message_id,..} if message_id==&queued.message.message_id)),"{ordering}: {facts:?}");
        let phase = ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase;
        if ordering == "before" {
            assert_eq!(phase, DurableTurnPhase::Completed);
            assert!(facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteered {scope,message_id,..} if scope.turn_id==original && message_id==&queued.message.message_id)));
        } else if ordering == "rejected" {
            assert_eq!(phase, DurableTurnPhase::Active);
            assert!(facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteerRejected {scope,message_id,receipt_id,reason} if scope.turn_id==original && message_id==&queued.message.message_id && receipt_id=="steer" && reason=="deliveryRejected")));
        } else {
            assert_eq!(phase, DurableTurnPhase::Failed);
            assert!(facts.iter().any(|fact| matches!(fact,ConversationActivityFact::PromptSteerRejected {scope,message_id,receipt_id,reason} if scope.turn_id==original && message_id==&queued.message.message_id && receipt_id=="steer" && reason=="deliveryUnprovable")));
        }
        assert!(runner.state.lock().unwrap().submitted.is_empty());
        assert_eq!(runner.state.lock().unwrap().starts, 1);
    }
}
