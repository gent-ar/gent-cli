use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::claude_control::ClaudePermissionRequest;
use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::claude_turn_options::ClaudeTurnOptions;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::lock::capture;
use gent_drivers::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, SandboxWorkspaceAccess,
};

#[derive(Default)]
struct State {
    output: Mutex<VecDeque<Vec<u8>>>,
    writes: Mutex<Vec<Vec<u8>>>,
}
#[derive(Clone)]
struct Process(Arc<State>);
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
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
        Ok(self.0.output.lock().unwrap().pop_front())
    }
}
struct Launcher(Arc<State>);
impl ProcessLauncher for Launcher {
    type Process = Process;
    fn launch(&self, _: &ProviderLaunch) -> Result<Process, SupervisorError> {
        Ok(Process(Arc::clone(&self.0)))
    }
}
fn start(root: &Path, mode: AgentChatMode) -> ClaudeRunStart {
    let executable = root.join("claude");
    std::fs::write(&executable, "test executable").unwrap();
    ClaudeRunStart {
        run_id: "run-1".into(),
        lock: capture("claude", &executable, "2.1.0", "entry").unwrap(),
        prompt: "hello".into(),
        content: Vec::new(),
        turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-haiku".into(),
            effort: AgentChatEffort::Low,
            mode,
        })
        .unwrap(),
        goal: None,
        fresh_context: None,
        intent: gent_drivers::LaunchIntent::Start,
        workspace_root: root.to_path_buf(),
        workspace_access: SandboxWorkspaceAccess::ReadOnly,
        mcp_config: None,
        selected_mcp_source_names: Vec::new(),
    }
}
fn runner(
    root: &Path,
    state: &Arc<State>,
    mode: AgentChatMode,
) -> ClaudeStreamRunner<Launcher, Process> {
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(state)),
        BufferPolicy::new(2, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner.start(start(root, mode)).unwrap();
    runner
}

const EXIT_PLAN: &[u8] = br#"{"type":"control_request","request_id":"plan-1","request":{"subtype":"can_use_tool","tool_use_id":"toolu-plan","tool_name":"ExitPlanMode","input":{"plan":"1. Add README.md"}}}
"#;

#[test]
fn a_plan_mode_exit_request_is_answered_as_submitted_for_review_not_user_gated() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state, AgentChatMode::Plan);
    state.output.lock().unwrap().push_back(EXIT_PLAN.to_vec());
    let effects = runner.poll("run-1").unwrap().unwrap_or_default();
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, ClaudeRunnerEffect::PermissionRequest(_)))
    );
    let writes = state.writes.lock().unwrap();
    let response: serde_json::Value = serde_json::from_slice(&writes[1]).unwrap();
    assert_eq!(response["response"]["request_id"], "plan-1");
    assert_eq!(response["response"]["response"]["behavior"], "deny");
    assert!(
        response["response"]["response"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("review"))
    );
}

#[test]
fn an_exit_plan_request_outside_plan_mode_stays_a_permission_request() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state, AgentChatMode::Agent);
    state.output.lock().unwrap().push_back(EXIT_PLAN.to_vec());
    assert_eq!(
        runner.poll("run-1").unwrap().unwrap(),
        vec![ClaudeRunnerEffect::PermissionRequest(
            ClaudePermissionRequest {
                request_id: "plan-1".into(),
                tool_use_id: "toolu-plan".into(),
                tool_name: "ExitPlanMode".into(),
                child_id: None,
            }
        )]
    );
}
