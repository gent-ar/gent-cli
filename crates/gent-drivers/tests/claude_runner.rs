use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::lock::capture;
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::NormalizedProviderEvent;

#[derive(Default)]
struct State {
    output: Mutex<VecDeque<Vec<u8>>>,
    writes: Mutex<Vec<Vec<u8>>>,
    exit: Mutex<Option<i32>>,
    signals: Mutex<Vec<ProcessTreeSignal>>,
}

#[derive(Clone)]
struct Process(Arc<State>);

impl ProcessTreeControl for Process {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.0.signals.lock().unwrap().push(signal);
        Ok(())
    }
}

impl ProviderProcess for Process {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.0.writes.lock().unwrap().push(frame.into());
        Ok(())
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.0.output.lock().unwrap().pop_front())
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(self.0.exit.lock().unwrap().map(Some))
    }
}

struct Launcher(Arc<State>);

impl ProcessLauncher for Launcher {
    type Process = Process;

    fn launch(&self, launch: &ProviderLaunch) -> Result<Process, SupervisorError> {
        assert_eq!(launch.provider, "claude");
        assert!(
            launch
                .arguments
                .windows(2)
                .any(|pair| pair == ["--output-format", "stream-json"])
        );
        Ok(Process(Arc::clone(&self.0)))
    }
}

fn start(run_id: &str, root: &Path, session: Option<&str>) -> ClaudeRunStart {
    let executable = root.join("claude");
    std::fs::write(&executable, "test executable").unwrap();
    ClaudeRunStart {
        run_id: run_id.into(),
        lock: capture("claude", &executable, "2.1.0", "entry").unwrap(),
        prompt: "hello".into(),
        resume_session_id: session.map(Into::into),
    }
}

#[test]
fn locked_claude_runner_writes_one_documented_prompt_and_normalizes_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, 128 * 1024, 0, 0).unwrap(),
    );
    runner
        .start(start("run-1", directory.path(), None))
        .unwrap();
    let input: serde_json::Value =
        serde_json::from_slice(&state.writes.lock().unwrap()[0]).unwrap();
    assert_eq!(input["type"], "user");
    assert!(input.get("session_id").is_none());
    state.output.lock().unwrap().push_back(
        br#"{"type":"system","subtype":"init","session_id":"private-session"}
{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}
"#
        .to_vec(),
    );
    let effects = runner.poll("run-1").unwrap().unwrap();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output { text, is_partial: false }))
            if text == "done"
    )));
}

#[test]
fn resume_binds_the_prompt_and_exit_drains_before_settlement() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, 64 * 1024, 0, 0).unwrap(),
    );
    runner
        .start(start("run-1", directory.path(), Some("private-session")))
        .unwrap();
    let input: serde_json::Value =
        serde_json::from_slice(&state.writes.lock().unwrap()[0]).unwrap();
    assert_eq!(input["session_id"], "private-session");
    *state.exit.lock().unwrap() = Some(0);
    assert_eq!(
        runner.poll("run-1").unwrap(),
        Some(vec![ClaudeRunnerEffect::Exited { code: Some(0) }])
    );
    assert!(runner.poll("run-1").is_err());
}
