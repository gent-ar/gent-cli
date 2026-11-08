use super::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use crate::buffering::BufferPolicy;
use crate::claude_turn_options::ClaudeTurnOptions;
use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::public_protocol::PublicWireFact;
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const REFRESH_CONTENTION: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Failed to refresh OAuth token: another Claude Code process is refreshing it or exited mid-refresh."}]},"parent_tool_use_id":null,"session_id":"s","uuid":"u","error":"server_error"}"#;
const SIGNED_OUT: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Login expired"}]},"parent_tool_use_id":null,"session_id":"s","uuid":"u","error":"authentication_failed"}"#;
const ERROR_RESULT: &str =
    r#"{"type":"result","subtype":"success","is_error":true,"result":"x","session_id":"s"}"#;
const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s"}"#;
const REPLY: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]},"session_id":"s"}
{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"s"}"#;

#[derive(Default)]
struct Fake {
    replies: Mutex<VecDeque<String>>,
    output: Mutex<VecDeque<Vec<u8>>>,
    writes: Mutex<Vec<Vec<u8>>>,
}

#[derive(Clone)]
struct Process(Arc<Fake>);

impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}

impl ProviderProcess for Process {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.0.writes.lock().unwrap().push(frame.into());
        if let Some(reply) = self.0.replies.lock().unwrap().pop_front() {
            self.0
                .output
                .lock()
                .unwrap()
                .push_back(format!("{INIT}\n{reply}\n").into_bytes());
        }
        Ok(())
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.0.output.lock().unwrap().pop_front())
    }
}

struct Launcher(Arc<Fake>);

impl ProcessLauncher for Launcher {
    type Process = Process;

    fn launch(&self, _: &ProviderLaunch) -> Result<Process, SupervisorError> {
        Ok(Process(Arc::clone(&self.0)))
    }
}

fn run(replies: &[String]) -> (Vec<ClaudeRunnerEffect>, Vec<Vec<u8>>) {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("claude");
    std::fs::write(&executable, "fake claude").unwrap();
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().extend(replies.iter().cloned());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&fake)),
        BufferPolicy::new(8, crate::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner.transient_retry_step = Duration::ZERO;
    runner
        .start(ClaudeRunStart {
            run_id: "run".into(),
            lock: crate::lock::capture("claude", &executable, "2.1.291", "entry").unwrap(),
            prompt: "hello".into(),
            content: Vec::new(),
            turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "claude-haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Agent,
            })
            .unwrap(),
            goal: None,
            fresh_context: None,
            intent: crate::LaunchIntent::Start,
            workspace_root: root.path().to_path_buf(),
            workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
            mcp_config: None,
        })
        .unwrap();
    let mut effects = Vec::new();
    for _ in 0..32 {
        effects.extend(runner.poll("run").unwrap().unwrap_or_default());
    }
    let writes = fake.writes.lock().unwrap().clone();
    (effects, writes)
}

fn failed(error: &str) -> String {
    format!("{error}\n{ERROR_RESULT}")
}

fn count(effects: &[ClaudeRunnerEffect], wanted: impl Fn(&PublicWireFact) -> bool) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, ClaudeRunnerEffect::Fact(fact) if wanted(fact)))
        .count()
}

fn retries(effects: &[ClaudeRunnerEffect]) -> usize {
    count(effects, |fact| {
        matches!(fact, PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic { classification })
            if classification == gent_types::PROVIDER_TRANSIENT_RETRY_DIAGNOSTIC)
    })
}

fn failures(effects: &[ClaudeRunnerEffect]) -> usize {
    count(effects, |fact| {
        matches!(
            fact,
            PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { .. })
                | PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Failed
                })
        )
    })
}

fn ready(effects: &[ClaudeRunnerEffect]) -> usize {
    count(effects, |fact| {
        fact == &PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready,
        })
    })
}

#[test]
fn refresh_contention_before_any_work_resends_the_same_prompt_until_claude_answers() {
    let contention = failed(REFRESH_CONTENTION);
    let (effects, writes) = run(&[contention.clone(), contention, REPLY.into()]);
    assert_eq!(writes.len(), 3);
    assert!(writes.iter().all(|write| write == &writes[0]));
    assert_eq!(retries(&effects), 2);
    assert_eq!(failures(&effects), 0);
    assert_eq!(ready(&effects), 1);
}

#[test]
fn contention_that_outlasts_the_retry_budget_fails_the_turn_once() {
    let contention = failed(REFRESH_CONTENTION);
    let (effects, writes) = run(&vec![contention; 5]);
    assert_eq!(writes.len(), 4);
    assert_eq!(retries(&effects), 3);
    assert_eq!(failures(&effects), 2);
    assert_eq!(ready(&effects), 0);
}

#[test]
fn a_signed_out_claude_is_never_retried() {
    let (effects, writes) = run(&[failed(SIGNED_OUT), REPLY.into()]);
    assert_eq!(writes.len(), 1);
    assert_eq!(retries(&effects), 0);
    assert_eq!(failures(&effects), 2);
}

#[test]
fn contention_after_claude_produced_output_is_never_retried() {
    let partial = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]},"session_id":"s"}"#;
    let (effects, writes) = run(&[
        format!("{partial}\n{}", failed(REFRESH_CONTENTION)),
        REPLY.into(),
    ]);
    assert_eq!(writes.len(), 1);
    assert_eq!(retries(&effects), 0);
    assert_eq!(failures(&effects), 2);
}
