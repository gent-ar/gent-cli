use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptLedger, AgentChatReadLedger, AgentChatWorkspaceLedger, Ledger,
    PendingPermissionLedger, PublicProviderResolver, PublicProviderRunError, PublicProviderRunner,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, NormalizedLifecycleSignal,
    NormalizedProviderEvent, ReceiptId, RunVersionLock, TurnPhase, WorkspaceRecord,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::authority_profile::{AuthorityProfileConfig, PublicDriverApproval, PublicDriverRequest};
use crate::claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptStart};
use crate::compatibility_assessment::CompatibilityAssessment;
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Clone, Default, Debug)]
pub(crate) struct Runner(pub(crate) Arc<Mutex<State>>);
#[derive(Default, Debug)]
pub(crate) struct State {
    pending: Option<String>,
    pub(crate) prepared_goals: Vec<Option<gent_types::GoalProjection>>,
    starts: usize,
    resumes: usize,
    pub(crate) effects: VecDeque<Vec<ClaudeRunnerEffect>>,
    pub(crate) poll_failure: bool,
    pub(crate) signals: Vec<gent_drivers::interrupt::ProcessTreeSignal>,
    pub(crate) permission_responses: Vec<(
        String,
        String,
        gent_drivers::claude_control::ClaudePermissionBehavior,
        bool,
    )>,
}

impl PublicProviderRunner for Runner {
    fn start(&self, _: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        assert_eq!(lock.provider, "claude");
        let mut state = self.0.lock().unwrap();
        assert!(state.pending.take().is_some());
        state.starts += 1;
        Ok(())
    }

    fn resume(&self, _: &str, _: &RunVersionLock, _: &str) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        assert!(state.pending.take().is_some());
        state.resumes += 1;
        Ok(())
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

impl ClaudePromptExecution for Runner {
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        prompt: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        state.pending = Some(run_id);
        state.prepared_goals.push(prompt.goal);
        Ok(())
    }
    fn cancel_claude_prompt(&self, _: &str) {
        self.0.lock().unwrap().pending = None;
    }
    fn poll_claude_prompt(
        &self,
        _: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        if state.poll_failure {
            return Err(PublicProviderRunError::Failed(
                "private runner detail".into(),
            ));
        }
        Ok(state.effects.pop_front())
    }
    fn signal_claude_process(
        &self,
        _: &str,
        signal: gent_drivers::interrupt::ProcessTreeSignal,
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().signals.push(signal);
        Ok(())
    }
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().permission_responses.push((
            run_id.into(),
            request_id.into(),
            behavior,
            persist_suggestions,
        ));
        Ok(())
    }
}

pub(crate) struct Resolver;
impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        (provider == "claude")
            .then(lock)
            .ok_or(PublicProviderRunError::CompatibilityDenied)
    }
}

#[path = "claude_prompt_lifecycle_test_support.rs"]
mod support;
pub(crate) use support::{compatibility, lock};

pub(crate) fn profile(
    compatibility: &CompatibilityAssessment,
) -> crate::authority_profile::ValidatedAuthorityProfile {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: "approved-evidence".into(),
            compatibility_manifest_sha256: compatibility.manifest_sha256().unwrap(),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap()
}

pub(crate) fn prompt(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    key: &str,
) -> gent_types::AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(format!("request-{key}")),
            receipt_id: ReceiptId(format!("receipt-{key}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            attachment_ids: vec![],
            tool_source_ids: vec![],
            text: format!("message-{key}"),
        })
        .unwrap();
    crate::readiness_test_support::release(ledger, &saved);
    saved
}

#[test]
fn standalone_claude_resumes_one_durable_conversation_and_relays_permission_to_owned_process() {
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
    let saved = prompt(&ledger, &conversation_id, "a");
    let runner = Runner::default();
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-session".into(),
        }),
        ClaudeRunnerEffect::PermissionRequest(
            gent_drivers::claude_control::ClaudePermissionRequest {
                request_id: "permission-a".into(),
                tool_use_id: "tool-a".into(),
                tool_name: "write_file".into(),
            },
        ),
    ]);
    runner.0.lock().unwrap().effects.push_back(vec![
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "done".into(),
            is_partial: false,
        })),
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )),
    ]);
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    let resumed = host.tick().unwrap();
    assert!(matches!(
        resumed.dispatch,
        Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
    ));
    let waiting = host.tick().unwrap();
    assert_eq!(waiting.batch.facts, 3);
    let pending = ledger
        .pending_permission(&conversation_id, &AgentChatRunId("run-a".into()))
        .unwrap()
        .expect("permission is durable before the waiting lifecycle facts are exposed");
    assert_eq!(pending.binding.decision_id.0, "permission-a");
    assert_eq!(pending.binding.turn_id, saved.message.turn_id);
    host.respond_permission(
        "run-a",
        "permission-a",
        gent_drivers::claude_control::ClaudePermissionBehavior::Allow,
        true,
    )
    .unwrap();
    assert_eq!(
        runner.0.lock().unwrap().permission_responses,
        [(
            "run-a".into(),
            "permission-a".into(),
            gent_drivers::claude_control::ClaudePermissionBehavior::Allow,
            true,
        )]
    );
    let ready = host.tick().unwrap();
    assert_eq!(ready.batch.facts, 2);
    assert!(
        host.needs_drive(),
        "a settled Claude prompt still owns a one-shot process until its exit is drained"
    );
    assert_eq!(
        ledger.find_run_session_binding("run-a").unwrap(),
        Some(gent_ports::RunSessionBinding {
            run_id: "run-a".into(),
            provider_session_id: "private-session".into(),
        }),
        "the daemon, rather than the CLI request, owns the resume identity"
    );
    let transcript = ledger
        .read_agent_chat_transcript(&conversation_id.0, None, 10)
        .unwrap();
    assert_eq!(transcript.conversation_id, conversation_id.0);
    assert!(transcript.events.iter().any(|event| event.text == "done"));
    prompt(&ledger, &conversation_id, "b");
    assert_eq!(
        host.tick().unwrap().dispatch,
        None,
        "ready process must remain bound until exit"
    );
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Exited { code: Some(0) }]);
    let resumed = host.tick().unwrap();
    assert!(matches!(
        resumed.dispatch,
        Some(crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Started { .. })
    ));
    let state = runner.0.lock().unwrap();
    assert_eq!((state.starts, state.resumes), (1, 1));
}
