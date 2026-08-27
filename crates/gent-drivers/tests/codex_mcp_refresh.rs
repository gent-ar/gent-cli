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
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, SandboxWorkspaceAccess,
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
    CodexPromptStart {
        working_directory: Some("/work".into()),
        workspace_root: root,
        workspace_access: SandboxWorkspaceAccess::ReadWrite,
        prompt: "next".into(),
        goal: None,
        fresh_context: None,
        turn_options: options(),
        attachments: vec![],
        selected_mcp_source_names: Vec::new(),
    }
}

#[test]
fn changed_mcp_config_terminates_the_old_process_and_fresh_start_uses_new_servers() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    std::fs::write(&executable, "test executable").unwrap();
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, r#"{"mcpServers":{"old":{"command":"old"}}}"#).unwrap();
    let state = Arc::new(State::default());
    let runner = CodexPromptRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, 65_536, 0, 0).unwrap(),
        None,
        Some(config.clone()),
    );
    let lock = capture("codex", &executable, "test", "test").unwrap();
    runner
        .prepare("run".into(), prompt(directory.path().into()))
        .unwrap();
    PublicProviderRunner::start(&runner, "run", &lock).unwrap();
    std::fs::write(&config, r#"{"mcpServers":{"new":{"command":"new"}}}"#).unwrap();
    assert!(runner.refresh_mcp_config("run").unwrap());
    assert_eq!(
        state.signals.lock().unwrap().as_slice(),
        &[ProcessTreeSignal::Terminate]
    );
    runner
        .prepare("run".into(), prompt(directory.path().into()))
        .unwrap();
    PublicProviderRunner::start(&runner, "run", &lock).unwrap();
    state
        .reads
        .lock()
        .unwrap()
        .push_back(b"{\"id\":1,\"result\":{}}\n".to_vec());
    runner.poll("run").unwrap();
    let frame: serde_json::Value =
        serde_json::from_slice(state.writes.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(frame["method"], "thread/start");
    assert_eq!(
        frame["params"]["config"]["mcp_servers"]["new"]["command"],
        "new"
    );
}
