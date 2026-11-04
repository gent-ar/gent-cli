use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome;
use crate::codex_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile, selection};
use crate::public_driver_runtime::PublicDriversRuntime;
use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{AgentChatPromptLedger, AgentChatSelectionLedger, AgentChatWorkspaceLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatRequestId, AgentChatRunId, AgentChatSelectionSwitch,
    CapabilitySet, ContextPolicy, HostEpoch, NormalizedLifecycleSignal, ReceiptId, TurnPhase,
    WorkspaceRecord,
};

pub(crate) struct Pool {
    pub(crate) ledger: SqliteLedger,
    pub(crate) runner: Runner,
    pub(crate) host: ApprovedCodexHost<SqliteLedger, Runner, Resolver>,
    prompts: u32,
}

impl Pool {
    pub(crate) fn new(capacity: usize) -> Self {
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
        let host = ApprovedCodexHost::new(runtime, "daemon-a".into(), HostEpoch(1), capacity);
        Self {
            ledger,
            runner,
            host,
            prompts: 0,
        }
    }

    pub(crate) fn conversation(&self, key: &str) -> AgentChatConversationId {
        let conversation_id = AgentChatConversationId(format!("conversation-{key}"));
        self.ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("create-{key}")),
                    idempotency_key: format!("create-{key}"),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation_id.clone(),
                    run_id: AgentChatRunId(format!("run-{key}")),
                    selection: selection(),
                },
                &WorkspaceRecord {
                    workspace_id: format!("workspace-{key}"),
                    canonical_path: format!("/workspace-{key}"),
                },
            )
            .unwrap();
        conversation_id
    }

    fn complete_turn(&mut self, conversation_id: &AgentChatConversationId, text: &str) -> String {
        let run_id = self.send(conversation_id, text);
        self.finish(&run_id);
        run_id
    }

    pub(crate) fn send(&mut self, conversation_id: &AgentChatConversationId, text: &str) -> String {
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
        let run_id = saved.run_id.0.clone();
        let dispatch = self.host.tick().unwrap().dispatch;
        assert!(
            matches!(dispatch, Some(CodexPromptDispatchOutcome::Started { .. })),
            "{text} was not started: {dispatch:?}"
        );
        run_id
    }

    pub(crate) fn finish(&mut self, run_id: &str) {
        self.runner
            .state
            .lock()
            .unwrap()
            .run_effects
            .entry(run_id.to_owned())
            .or_default()
            .push_back(vec![
                CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
                    provider_session_id: format!("thread-{run_id}"),
                }),
                CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::RootPhase {
                        phase: TurnPhase::Ready,
                    },
                )),
            ]);
        self.host.tick().unwrap();
    }

    pub(crate) fn counts(&self) -> (usize, Vec<String>, Vec<String>) {
        let state = self.runner.state.lock().unwrap();
        (
            state.starts,
            state.releases.clone(),
            state.submitted.clone(),
        )
    }
}

#[test]
fn a_second_conversation_keeps_the_first_conversations_process_warm() {
    let mut pool = Pool::new(4);
    let first = pool.conversation("a");
    let second = pool.conversation("b");

    pool.complete_turn(&first, "a1");
    pool.complete_turn(&second, "b1");
    pool.complete_turn(&first, "a2");
    pool.complete_turn(&second, "b2");

    let (starts, releases, submitted) = pool.counts();
    assert_eq!(starts, 2, "each conversation launches its process once");
    assert!(
        releases.is_empty(),
        "no warm process is stopped: {releases:?}"
    );
    assert_eq!(submitted, ["a2", "b2"]);
}

#[test]
fn a_full_pool_stops_only_the_least_recently_used_idle_process() {
    let mut pool = Pool::new(2);
    let first = pool.conversation("a");
    let second = pool.conversation("b");
    let third = pool.conversation("c");

    pool.complete_turn(&first, "a1");
    pool.complete_turn(&second, "b1");
    pool.complete_turn(&first, "a2");
    pool.complete_turn(&third, "c1");
    pool.complete_turn(&first, "a3");

    let (starts, releases, submitted) = pool.counts();
    assert_eq!(releases, ["run-b"]);
    assert_eq!(starts, 3);
    assert_eq!(submitted, ["a2", "a3"]);
}

#[test]
fn a_selection_switch_stops_the_superseded_run_but_not_other_conversations() {
    let mut pool = Pool::new(4);
    let first = pool.conversation("a");
    let second = pool.conversation("b");
    pool.complete_turn(&first, "a1");
    pool.complete_turn(&second, "b1");
    let mut switched = selection();
    switched.model = "gpt-5.6-terra".into();
    pool.ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch-receipt".into()),
            idempotency_key: "switch-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: first.clone(),
            parent_run_id: AgentChatRunId("run-a".into()),
            run_id: AgentChatRunId("run-a-child".into()),
            selection: switched,
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();

    assert_eq!(pool.complete_turn(&first, "a2"), "run-a-child");
    pool.complete_turn(&second, "b2");

    let (starts, releases, submitted) = pool.counts();
    assert_eq!(releases, ["run-a"]);
    assert_eq!(starts, 3);
    assert_eq!(submitted, ["b2"]);
}
