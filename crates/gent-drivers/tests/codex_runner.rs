use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::codex_runner::{CodexAppServerRunner, CodexRunStart, CodexRunnerEffect};
use gent_drivers::codex_session::CodexSessionConfig;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::lock::capture;
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};

#[derive(Default)]
struct State {
    output: Mutex<VecDeque<Vec<u8>>>,
    writes: Mutex<Vec<Vec<u8>>>,
    write_fails: Mutex<bool>,
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
        if *self.0.write_fails.lock().unwrap() {
            return Err(ProcessTreeError::Failed("test write failure".into()));
        }
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
        assert_eq!(launch.provider, "codex");
        assert_eq!(launch.arguments, ["app-server"]);
        Ok(Process(Arc::clone(&self.0)))
    }
}

fn start(run_id: &str, root: &Path) -> CodexRunStart {
    let executable = root.join("codex");
    std::fs::write(&executable, "test executable").unwrap();
    CodexRunStart {
        run_id: run_id.into(),
        lock: capture("codex", &executable, "0.144.1", "entry").unwrap(),
        session: CodexSessionConfig {
            working_directory: Some("/work".into()),
            resume_thread_id: None,
        },
        prompt: "hello".into(),
    }
}

fn method(frame: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(&frame[..frame.len() - 1]).unwrap()["method"]
        .as_str()
        .unwrap()
        .into()
}

#[test]
fn owned_process_handshake_is_bounded_and_yields_only_normalized_facts() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = CodexAppServerRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, 128 * 1024, 0, 0).unwrap(),
    );
    runner.start(start("run-1", directory.path())).unwrap();
    assert_eq!(method(&state.writes.lock().unwrap()[0]), "initialize");

    state.output.lock().unwrap().push_back(
        br#"{"id":1,"result":{}}
"#
        .to_vec(),
    );
    assert_eq!(runner.poll("run-1").unwrap(), None);
    assert_eq!(
        state.writes.lock().unwrap()[1..]
            .iter()
            .map(|frame| method(frame))
            .collect::<Vec<_>>(),
        ["initialized", "thread/start"]
    );

    state.output.lock().unwrap().push_back(
        br#"{"method":"thread/started","params":{"thread":{"id":"private-thread"}}}
"#
        .to_vec(),
    );
    assert!(matches!(
        runner.poll("run-1").unwrap(),
        Some(effects) if matches!(effects.as_slice(), [CodexRunnerEffect::Fact(PublicWireFact::SessionStarted { .. })])
    ));
    state.output.lock().unwrap().push_back(
        br#"{"id":2,"result":{"thread":{"id":"private-thread"}}}
"#
        .to_vec(),
    );
    assert_eq!(runner.poll("run-1").unwrap(), None);
    assert_eq!(
        method(state.writes.lock().unwrap().last().unwrap()),
        "turn/start"
    );

    *state.exit.lock().unwrap() = Some(0);
    assert_eq!(
        runner.poll("run-1").unwrap(),
        Some(vec![CodexRunnerEffect::Exited { code: Some(0) }])
    );
}

#[test]
fn tree_signals_stay_owned_by_the_runner() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = CodexAppServerRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, 64 * 1024, 0, 0).unwrap(),
    );
    runner.start(start("run-1", directory.path())).unwrap();
    runner
        .signal("run-1", ProcessTreeSignal::Interrupt)
        .unwrap();
    assert_eq!(
        state.signals.lock().unwrap().as_slice(),
        &[ProcessTreeSignal::Interrupt]
    );
}

#[test]
fn failed_initial_write_terminates_the_new_process_tree() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    *state.write_fails.lock().unwrap() = true;
    let mut runner = CodexAppServerRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, 64 * 1024, 0, 0).unwrap(),
    );
    assert!(runner.start(start("run-1", directory.path())).is_err());
    assert_eq!(
        state.signals.lock().unwrap().as_slice(),
        &[ProcessTreeSignal::Terminate]
    );
}
