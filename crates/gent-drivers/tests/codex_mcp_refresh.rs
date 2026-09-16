use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gent_drivers::{
    buffering::BufferPolicy,
    codex_prompt_runner::{CodexPromptRunner, CodexPromptStart},
    codex_session::CodexTurnOptions,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    lock::capture,
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError},
};
use gent_ports::PublicProviderRunner;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, RunVersionLock,
    SandboxWorkspaceAccess,
};

#[derive(Default)]
struct State {
    writes: Mutex<Vec<Vec<u8>>>,
    reads: Mutex<VecDeque<Vec<u8>>>,
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
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.0.reads.lock().unwrap().pop_front())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(None)
    }
}
struct Launcher(Arc<State>);
impl ProcessLauncher for Launcher {
    type Process = Process;
    fn launch(&self, _: &ProviderLaunch) -> Result<Process, SupervisorError> {
        Ok(Process(Arc::clone(&self.0)))
    }
}

fn options() -> CodexTurnOptions {
    CodexTurnOptions::from_selection(
        &AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
        Some("/work"),
    )
    .unwrap()
}
fn prompt(root: PathBuf) -> CodexPromptStart {
    prompt_with_servers(root, None)
}

fn prompt_with_servers(root: PathBuf, mcp_servers: Option<serde_json::Value>) -> CodexPromptStart {
    CodexPromptStart {
        working_directory: Some("/work".into()),
        workspace_root: root,
        workspace_access: SandboxWorkspaceAccess::ReadWrite,
        prompt: "next".into(),
        goal: None,
        fresh_context: None,
        turn_options: options(),
        attachments: vec![],
        mcp_servers,
        interrupted_reply: None,
    }
}

fn runner(directory: &std::path::Path, state: &Arc<State>) -> CodexPromptRunner<Launcher, Process> {
    std::fs::write(
        directory.join("mcp.json"),
        r#"{"mcpServers":{"old":{"command":"old"}}}"#,
    )
    .unwrap();
    CodexPromptRunner::new(
        Launcher(Arc::clone(state)),
        BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
        None,
        Some(directory.join("mcp.json")),
    )
}

fn started(
    directory: &std::path::Path,
    runner: &CodexPromptRunner<Launcher, Process>,
) -> RunVersionLock {
    let executable = directory.join("codex");
    std::fs::write(&executable, "test executable").unwrap();
    let lock = capture("codex", &executable, "test", "test").unwrap();
    runner
        .prepare("run".into(), prompt(directory.into()))
        .unwrap();
    PublicProviderRunner::start(runner, "run", &lock).unwrap();
    lock
}

#[test]
fn changed_mcp_config_replaces_the_process_and_resumes_the_bound_thread_with_new_servers() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let runner = runner(directory.path(), &state);
    let lock = started(directory.path(), &runner);
    assert!(!runner.refresh_mcp_config("run").unwrap());
    std::fs::write(
        directory.path().join("mcp.json"),
        r#"{"mcpServers":{"new":{"command":"new"}}}"#,
    )
    .unwrap();
    assert!(runner.refresh_mcp_config("run").unwrap());
    assert_eq!(
        state.signals.lock().unwrap().as_slice(),
        &[ProcessTreeSignal::Terminate]
    );
    runner
        .prepare(
            "run".into(),
            prompt_with_servers(
                directory.path().into(),
                runner
                    .current_mcp_servers()
                    .unwrap()
                    .map(|(servers, _)| servers),
            ),
        )
        .unwrap();
    PublicProviderRunner::resume(&runner, "run", &lock, "bound-thread").unwrap();
    state
        .reads
        .lock()
        .unwrap()
        .push_back(b"{\"id\":1,\"result\":{}}\n".to_vec());
    runner.poll("run").unwrap();
    let frame: serde_json::Value =
        serde_json::from_slice(state.writes.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(frame["method"], "thread/resume");
    assert_eq!(frame["params"]["threadId"], "bound-thread");
    assert_eq!(frame["params"]["excludeTurns"], true);
    assert_eq!(
        frame["params"]["config"]["mcp_servers"]["new"]["command"],
        "new"
    );
}

#[test]
fn changed_mcp_config_after_the_process_is_gone_leaves_nothing_to_replace() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let runner = runner(directory.path(), &state);
    started(directory.path(), &runner);
    runner.release_session("run").unwrap();
    std::fs::write(
        directory.path().join("mcp.json"),
        r#"{"mcpServers":{"new":{"command":"new"}}}"#,
    )
    .unwrap();
    assert!(!runner.refresh_mcp_config("run").unwrap());
    assert_eq!(
        state.signals.lock().unwrap().as_slice(),
        &[ProcessTreeSignal::Terminate]
    );
}
