use gent_drivers::{claude_runner::ClaudeRunnerEffect, public_protocol::PublicWireFact};
use gent_ports::{
    AgentChatPromptLedger, AgentChatReadLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, ConversationLedger, PendingPermissionLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, ConversationActivityFact, DurableTurnPhase,
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    ReceiptId, TurnPhase, WorkspaceRecord,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle_tests::{Runner, compatibility, profile, prompt};
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn claude_poll_failure_fails_only_that_turn_and_releases_its_process() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "hello".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        crate::claude_prompt_lifecycle_tests::Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    runner.0.lock().unwrap().poll_failure = true;

    assert_eq!(host.tick().unwrap().batch.exited_runs, 1);
    assert_eq!(
        ledger.list_run_turns("run-a").unwrap()[0].phase,
        DurableTurnPhase::Failed
    );
    assert!(
        ledger
            .read_conversation_activity_page(&conversation_id.0, "run-a", 0, 64)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(
                fact,
                ConversationActivityFact::Terminal { scope, phase: TurnPhase::Failed, .. }
                    if scope.turn_id == saved.message.turn_id
            ))
    );
    assert!(!runner.0.lock().unwrap().active);
    assert_eq!(
        host.signal_active(gent_drivers::interrupt::ProcessTreeSignal::Interrupt)
            .unwrap(),
        0
    );
}

#[test]
fn accepted_user_interrupt_settles_claude_as_interrupted_instead_of_failed() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "hello".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &saved);
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        crate::claude_prompt_lifecycle_tests::Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "session-a".into(),
        }),
        ClaudeRunnerEffect::PermissionRequest(
            gent_drivers::claude_control::ClaudePermissionRequest {
                request_id: "permission-a".into(),
                tool_use_id: "tool-a".into(),
                tool_name: "write_file".into(),
                child_id: None,
            },
        ),
    ]);
    host.tick().unwrap();
    assert!(
        ledger
            .pending_permission(&conversation_id, &AgentChatRunId("run-a".into()))
            .unwrap()
            .is_some()
    );
    host.interrupt("run-a").unwrap();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::Provider,
                message: "Claude ended the turn with an error.".into(),
            },
        )),
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Failed,
            },
        )),
        ClaudeRunnerEffect::Exited { code: Some(130) },
    ]);

    host.tick().unwrap();

    assert_eq!(
        ledger
            .list_run_turns("run-a")
            .unwrap()
            .last()
            .unwrap()
            .phase,
        DurableTurnPhase::Interrupted
    );
    assert!(
        ledger
            .pending_permission(&conversation_id, &AgentChatRunId("run-a".into()))
            .unwrap()
            .is_none()
    );
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(
                fact,
                ConversationActivityFact::Terminal {
                    phase: TurnPhase::Interrupted,
                    ..
                }
            ))
    );
    assert!(
        ledger
            .read_agent_chat_transcript(&conversation_id.0, None, 32)
            .unwrap()
            .events
            .iter()
            .all(|event| event.text != "Claude ended the turn with an error.")
    );
}

#[test]
fn interrupted_claude_process_exits_before_a_queued_prompt_is_released() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    prompt(&ledger, &conversation_id, "first");
    let runner = Runner::default();
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        crate::claude_prompt_lifecycle_tests::Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "session-a".into(),
            },
        )]);
    host.tick().unwrap();
    let queued = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-queued".into()),
            receipt_id: ReceiptId("receipt-queued".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "queued".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &queued);
    host.interrupt("run-a").unwrap();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Failed,
            },
        ))]);

    let terminal = host.tick().unwrap();
    assert!(terminal.dispatch.is_none());
    assert_eq!(
        ledger.list_run_turns("run-a").unwrap()[1].phase,
        DurableTurnPhase::Active
    );

    runner.0.lock().unwrap().active = false;
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Exited { code: Some(130) }]);
    let released = host.tick().unwrap();
    assert!(matches!(
        released.dispatch,
        Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
    ));
    assert_eq!(
        ledger.list_run_turns("run-a").unwrap()[1].phase,
        DurableTurnPhase::Active
    );
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "session-a".into(),
            },
        )]);
    host.tick().unwrap();
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes), (1, 1));
}

#[test]
fn send_now_interrupts_only_the_active_turn_and_releases_the_queued_prompt_once() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = claude_conversation(&ledger);
    let active = prompt(&ledger, &conversation_id, "active");
    let runner = Runner::default();
    let push = |effects: Vec<ClaudeRunnerEffect>| {
        runner.0.lock().unwrap().effects.push_back(effects);
    };
    let session_started = || {
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "session-a".into(),
        })
    };
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        crate::claude_prompt_lifecycle_tests::Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    push(vec![session_started()]);
    host.tick().unwrap();
    let queued = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-send-now".into()),
            receipt_id: ReceiptId("receipt-send-now".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Queue,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "send now".into(),
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &queued);
    assert!(host.tick().unwrap().dispatch.is_none());

    host.interrupt("run-a").unwrap();
    push(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Failed,
            },
        )),
        ClaudeRunnerEffect::Exited { code: Some(130) },
    ]);
    runner.0.lock().unwrap().active = false;
    let started = |tick: crate::claude_prompt_lifecycle::ClaudeLifecycleTick| {
        matches!(
            tick.dispatch,
            Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
        )
    };
    assert!(started(host.tick().unwrap()));
    push(vec![session_started()]);
    assert!(!started(host.tick().unwrap()));
    assert!(!started(host.tick().unwrap()));

    let turns = ledger.list_run_turns("run-a").unwrap();
    let phase = |turn_id: &str| {
        turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .unwrap()
            .phase
    };
    assert_eq!(
        phase(&active.message.turn_id),
        DurableTurnPhase::Interrupted
    );
    assert_eq!(phase(&queued.message.turn_id), DurableTurnPhase::Active);
    let (signals, starts, resumes) = {
        let state = runner.0.lock().unwrap();
        (state.signals.clone(), state.starts, state.resumes)
    };
    assert_eq!(
        signals,
        [gent_drivers::interrupt::ProcessTreeSignal::Interrupt]
    );
    assert_eq!((starts, resumes), (1, 1));
    let activity = ledger
        .read_conversation_activity_page(&conversation_id.0, "run-a", 0, 64)
        .unwrap()
        .facts;
    let released = activity.iter().filter(|fact| {
        matches!(fact, ConversationActivityFact::PromptReleased { message_id, .. } if message_id == &queued.message.message_id)
    });
    assert_eq!(released.count(), 1);
    assert!(activity.iter().all(|fact| !matches!(
        fact,
        ConversationActivityFact::Terminal { scope, .. } if scope.turn_id == queued.message.turn_id
    )));
}

fn claude_conversation(ledger: &SqliteLedger) -> AgentChatConversationId {
    let conversation_id = AgentChatConversationId("conversation-a".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
    conversation_id
}
