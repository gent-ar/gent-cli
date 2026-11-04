use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptLedger, AgentChatWorkspaceLedger, PublicProviderRunError, PublicProviderRunner,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch, NormalizedLifecycleSignal,
    ReceiptId, RunVersionLock, TurnPhase, WorkspaceRecord,
};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::claude_prompt_lifecycle::{
    ClaudePromptDispatchOutcome, ClaudePromptExecution, ClaudePromptStart,
};
use crate::claude_prompt_lifecycle_tests::{Resolver, compatibility, profile};
use crate::public_driver_runtime::PublicDriversRuntime;

#[derive(Default, Debug)]
struct State {
    pending: BTreeSet<String>,
    sessions: BTreeSet<String>,
    effects: BTreeMap<String, VecDeque<Vec<ClaudeRunnerEffect>>>,
    starts: Vec<String>,
    releases: Vec<String>,
    submitted: Vec<String>,
}

#[derive(Clone, Default, Debug)]
struct Runner(Arc<Mutex<State>>);

impl PublicProviderRunner for Runner {
    fn start(&self, run_id: &str, _: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        assert!(state.pending.remove(run_id));
        state.starts.push(run_id.into());
        state.sessions.insert(run_id.into());
        Ok(())
    }

    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        _: &str,
    ) -> Result<(), PublicProviderRunError> {
        self.start(run_id, lock)
    }

    fn interrupt(&self, _: &str) -> Result<(), PublicProviderRunError> {
        Ok(())
    }
}

impl ClaudePromptExecution for Runner {
    fn prepare_claude_prompt(
        &self,
        run_id: String,
        _: ClaudePromptStart,
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().pending.insert(run_id);
        Ok(())
    }

    fn cancel_claude_prompt(&self, run_id: &str) {
        self.0.lock().unwrap().pending.remove(run_id);
    }

    fn poll_claude_prompt(
        &self,
        run_id: &str,
    ) -> Result<Option<Vec<ClaudeRunnerEffect>>, PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        Ok(state.effects.get_mut(run_id).and_then(VecDeque::pop_front))
    }

    fn has_claude_session(&self, run_id: &str) -> bool {
        self.0.lock().unwrap().sessions.contains(run_id)
    }

    fn release_claude_session(&self, run_id: &str) -> Result<(), PublicProviderRunError> {
        let mut state = self.0.lock().unwrap();
        state.sessions.remove(run_id);
        state.releases.push(run_id.into());
        Ok(())
    }

    fn submit_claude_prompt(
        &self,
        _: &str,
        prompt: &str,
        _: Option<&gent_types::GoalProjection>,
        _: &[serde_json::Value],
    ) -> Result<(), PublicProviderRunError> {
        self.0.lock().unwrap().submitted.push(prompt.into());
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

struct Pool {
    ledger: SqliteLedger,
    runner: Runner,
    host: ApprovedClaudeHost<SqliteLedger, Runner, Resolver>,
    prompts: u32,
}

impl Pool {
    fn new(capacity: usize) -> Self {
        let ledger = SqliteLedger::in_memory().unwrap();
        let runner = Runner::default();
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
        let host =
            ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), capacity, None);
        Self {
            ledger,
            runner,
            host,
            prompts: 0,
        }
    }

    fn conversation(&self, key: &str) -> AgentChatConversationId {
        let conversation_id = AgentChatConversationId(format!("conversation-{key}"));
        self.ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("create-{key}")),
                    idempotency_key: format!("create-{key}"),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation_id.clone(),
                    run_id: AgentChatRunId(format!("run-{key}")),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-test".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: format!("workspace-{key}"),
                    canonical_path: format!("/workspace-{key}"),
                },
            )
            .unwrap();
        conversation_id
    }

    fn complete_turn(&mut self, conversation_id: &AgentChatConversationId, text: &str) {
        self.prompts += 1;
        let id = format!("prompt-{}", self.prompts);
        let saved = self
            .ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId(id.clone()),
                receipt_id: ReceiptId(format!("{id}-receipt")),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: text.into(),
            })
            .unwrap();
        crate::readiness_test_support::release(&self.ledger, &saved);
        let dispatch = self.host.tick().unwrap().dispatch;
        assert!(
            matches!(dispatch, Some(ClaudePromptDispatchOutcome::Started { .. })),
            "{text} was not started: {dispatch:?}"
        );
        let run_id = saved.run_id.0;
        let ready = vec![
            ClaudeRunnerEffect::Fact(PublicWireFact::SessionStarted {
                provider_session_id: format!("session-{run_id}"),
            }),
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                },
            )),
        ];
        self.runner
            .0
            .lock()
            .unwrap()
            .effects
            .entry(run_id)
            .or_default()
            .push_back(ready);
        self.host.tick().unwrap();
    }
}

#[test]
fn alternating_claude_conversations_reuse_each_warm_process() {
    let mut pool = Pool::new(4);
    let first = pool.conversation("a");
    let second = pool.conversation("b");

    pool.complete_turn(&first, "a1");
    pool.complete_turn(&second, "b1");
    pool.complete_turn(&first, "a2");
    pool.complete_turn(&second, "b2");

    let state = pool.runner.0.lock().unwrap();
    assert_eq!(state.starts, ["run-a", "run-b"]);
    assert!(state.releases.is_empty(), "{:?}", state.releases);
    assert_eq!(state.submitted, ["a2", "b2"]);
}

#[test]
fn a_full_claude_pool_stops_only_the_least_recently_used_process() {
    let mut pool = Pool::new(2);
    let first = pool.conversation("a");
    let second = pool.conversation("b");
    let third = pool.conversation("c");

    pool.complete_turn(&first, "a1");
    pool.complete_turn(&second, "b1");
    pool.complete_turn(&first, "a2");
    pool.complete_turn(&third, "c1");
    pool.complete_turn(&first, "a3");

    let state = pool.runner.0.lock().unwrap();
    assert_eq!(state.releases, ["run-b"]);
    assert_eq!(state.starts, ["run-a", "run-b", "run-c"]);
    assert_eq!(state.submitted, ["a2", "a3"]);
}
