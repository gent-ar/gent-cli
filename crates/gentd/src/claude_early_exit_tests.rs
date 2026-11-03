use crate::claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptStart};
use gent_drivers::{claude_runner::ClaudeRunnerEffect, public_protocol::PublicWireFact};
use gent_ports::{
    AgentChatPromptLedger, AgentChatReadLedger, AgentChatWorkspaceLedger, ConversationLedger,
    PublicProviderResolver, PublicProviderRunError, PublicProviderRunner,
};
use gent_types::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::claude_prompt_lifecycle_tests as compatibility;
use crate::readiness_test_support as readiness;

#[derive(Clone, Default)]
struct Runner(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    active: bool,
    start_failure: bool,
    effects: VecDeque<Vec<ClaudeRunnerEffect>>,
}
impl PublicProviderRunner for Runner {
    fn start(&self, _: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        if state.start_failure {
            return Err(PublicProviderRunError::Failed(
                "Contained MCP configuration rejected before provider startup.".into(),
            ));
        }
        state.active = true;
        Ok(())
    }
    fn resume(
        &self,
        run: &str,
        lock: &RunVersionLock,
        _: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.start(run, lock)
    }
    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}
impl ClaudePromptExecution for Runner {
    fn prepare_claude_prompt(
        &self,
        _: String,
        _: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
    fn cancel_claude_prompt(&self, _: &str) {}
    fn poll_claude_prompt(
        &self,
        _: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        Ok(self.0.lock().unwrap().effects.pop_front())
    }
    fn has_claude_session(&self, _: &str) -> bool {
        self.0.lock().unwrap().active
    }
    fn release_claude_session(&self, _: &str) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().active = false;
        Ok(())
    }
    fn submit_claude_prompt(
        &self,
        _: &str,
        _: &str,
        _: Option<&gent_types::GoalProjection>,
        _: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
    fn steer_claude_prompt(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
    fn signal_claude_process(
        &self,
        _: &str,
        _: gent_drivers::interrupt::ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
    fn respond_claude_permission(
        &self,
        _: &str,
        _: &str,
        _: gent_drivers::claude_control::ClaudePermissionBehavior,
        _: bool,
    ) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}
struct Resolver;
impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        assert_eq!(provider, "claude");
        Ok(compatibility::lock())
    }
}

fn scenario(
    batches: Vec<Vec<ClaudeRunnerEffect>>,
    interrupt: bool,
) -> (Vec<String>, DurableTurnPhase) {
    scenario_checked(batches, interrupt, false)
}

fn scenario_checked(
    batches: Vec<Vec<ClaudeRunnerEffect>>,
    interrupt: bool,
    check_fences: bool,
) -> (Vec<String>, DurableTurnPhase) {
    scenario_start_checked(batches, interrupt, check_fences, false)
}

fn scenario_start_checked(
    batches: Vec<Vec<ClaudeRunnerEffect>>,
    interrupt: bool,
    check_fences: bool,
    start_failure: bool,
) -> (Vec<String>, DurableTurnPhase) {
    let ledger = gent_store::SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("early-exit-conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("early-exit-run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "early-exit-workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: "fixed synthetic prompt".into(),
        })
        .unwrap();
    readiness::release(&ledger, &saved);
    let runner = Runner::default();
    runner.0.lock().unwrap().start_failure = start_failure;
    let assessment = compatibility::compatibility();
    let profile = crate::authority_profile::AuthorityProfileConfig {
        public_drivers: crate::authority_profile::PublicDriverRequest::Approved(
            crate::authority_profile::PublicDriverApproval {
                evidence_reference: "fixture".into(),
                compatibility_manifest_sha256: assessment.manifest_sha256().unwrap(),
            },
        ),
        ..Default::default()
    }
    .validate()
    .unwrap();
    let runtime = crate::public_driver_runtime::PublicDriversRuntime::new(
        profile,
        gent_runtime::Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        assessment,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = crate::approved_claude_host::ApprovedClaudeHost::new(
        runtime,
        "early-exit-owner".into(),
        HostEpoch(1),
        1,
        None,
    );
    host.tick().unwrap();
    if check_fences {
        use gent_ports::AgentChatPromptDispatchLedger;
        assert!(
            ledger
                .fail_agent_chat_prompt_prelaunch(
                    &saved.message.message_id,
                    "wrong-owner",
                    HostEpoch(1),
                    "not authorized",
                    None
                )
                .is_err()
        );
        assert!(
            ledger
                .fail_agent_chat_prompt_prelaunch(
                    &saved.message.message_id,
                    "early-exit-owner",
                    HostEpoch(2),
                    "stale epoch",
                    None
                )
                .is_err()
        );
    }
    if interrupt {
        host.interrupt_turn("early-exit-run", &saved.message.turn_id)
            .unwrap();
    }
    for (index, batch) in batches.into_iter().enumerate() {
        runner.0.lock().unwrap().effects.push_back(batch);
        host.tick().unwrap();
        if check_fences && index == 0 {
            use gent_ports::AgentChatPromptDispatchLedger;
            assert!(
                ledger
                    .fail_agent_chat_prompt_prelaunch(
                        &saved.message.message_id,
                        "early-exit-owner",
                        HostEpoch(1),
                        "announced session is not prelaunch",
                        None
                    )
                    .is_err()
            );
        }
    }
    let messages = ledger
        .read_agent_chat_transcript(&conversation.0, None, 64)
        .unwrap()
        .events
        .into_iter()
        .map(|event| event.text)
        .collect();
    let phase = ledger.list_run_turns("early-exit-run").unwrap()[0].phase;
    (messages, phase)
}
fn started() -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
        provider_session_id: "fixture-session".into(),
    })
}
fn phase(phase: TurnPhase) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase { phase },
    ))
}

#[test]
fn exit_before_session_has_durable_code_only_reason() {
    let (messages, state) = scenario(
        vec![vec![ClaudeRunnerEffect::Exited { code: Some(125) }]],
        false,
    );
    assert_eq!(state, DurableTurnPhase::Failed);
    assert!(
        messages
            .iter()
            .any(|text| text.contains("process status 125"))
    );
}
#[test]
fn announced_session_exit_also_explains_missing_provider_failure() {
    let (messages, state) = scenario(
        vec![
            vec![started()],
            vec![ClaudeRunnerEffect::Exited { code: Some(1) }],
        ],
        false,
    );
    assert_eq!(state, DurableTurnPhase::Failed);
    assert!(
        messages
            .iter()
            .any(|text| text.contains("process status 1"))
    );
}
#[test]
fn a_real_provider_failure_is_not_replaced_by_the_exit_notice() {
    let (messages, state) = scenario(
        vec![
            vec![
                started(),
                ClaudeRunnerEffect::Fact(PublicWireFact::Event(
                    NormalizedProviderEvent::ProviderFailure {
                        classification: ProviderFailureClassification::Provider,
                        message: "specific fixture failure".into(),
                    },
                )),
            ],
            vec![ClaudeRunnerEffect::Exited { code: Some(1) }],
        ],
        false,
    );
    assert_eq!(state, DurableTurnPhase::Failed);
    assert!(
        messages
            .iter()
            .any(|text| text.contains("specific fixture failure"))
    );
    assert!(messages.iter().all(|text| !text.contains("process status")));
}
#[test]
fn completed_or_interrupted_turns_do_not_get_failure_notices() {
    let (messages, state) = scenario(
        vec![
            vec![started(), phase(TurnPhase::Ready)],
            vec![ClaudeRunnerEffect::Exited { code: Some(0) }],
        ],
        false,
    );
    assert_eq!(state, DurableTurnPhase::Completed);
    assert!(messages.iter().all(|text| !text.contains("process status")));
    let (messages, state) = scenario(
        vec![vec![ClaudeRunnerEffect::Exited { code: Some(130) }]],
        true,
    );
    assert_eq!(state, DurableTurnPhase::Interrupted);
    assert!(messages.iter().all(|text| !text.contains("process status")));
}

#[test]
fn owner_epoch_and_existing_session_guards_remain_closed() {
    let (messages, state) = scenario_checked(
        vec![
            vec![started()],
            vec![ClaudeRunnerEffect::Exited { code: Some(1) }],
        ],
        false,
        true,
    );
    assert_eq!(state, DurableTurnPhase::Failed);
    for forbidden in [
        "not authorized",
        "stale epoch",
        "announced session is not prelaunch",
    ] {
        assert!(messages.iter().all(|text| !text.contains(forbidden)));
    }
}

#[test]
fn pre_spawn_mcp_rejection_has_a_notice_without_replaying_the_prompt() {
    let (messages, phase) = scenario_start_checked(vec![], false, false, true);
    assert_eq!(phase, DurableTurnPhase::Failed);
    assert!(messages.iter().any(|text| {
        text.contains("Contained MCP configuration rejected before provider startup.")
    }));
}
