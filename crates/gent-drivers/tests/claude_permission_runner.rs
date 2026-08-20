use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::claude_turn_options::ClaudeTurnOptions;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::lock::capture;
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    NormalizedLifecycleSignal, NormalizedProviderEvent, SandboxWorkspaceAccess, ToolPhase,
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
fn start(root: &Path) -> ClaudeRunStart {
    let executable = root.join("claude");
    std::fs::write(&executable, "test executable").unwrap();
    ClaudeRunStart {
        run_id: "run-1".into(),
        lock: capture("claude", &executable, "2.1.0", "entry").unwrap(),
        prompt: "hello".into(),
        turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-haiku".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        })
        .unwrap(),
        goal: None,
        fresh_context: None,
        resume_session_id: None,
        workspace_root: root.to_path_buf(),
        workspace_access: SandboxWorkspaceAccess::ReadOnly,
    }
}
fn runner(root: &Path, state: &Arc<State>) -> ClaudeStreamRunner<Launcher, Process> {
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(state)),
        BufferPolicy::new(2, 128 * 1024, 0, 0).unwrap(),
    );
    runner.start(start(root)).unwrap();
    runner
}

#[test]
fn permission_request_retains_suggestions_privately_and_writes_only_its_response() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    state.output.lock().unwrap().push_back(br#"{"type":"control_request","request_id":"request-1","request":{"subtype":"can_use_tool","tool_use_id":"tool-1","tool_name":"Bash","input":{"command":"private command"},"permission_suggestions":[{"type":"addDirectories","path":"/private"}]}}
"#.to_vec());
    let effects = runner.poll("run-1").unwrap().unwrap();
    assert_eq!(
        effects,
        vec![ClaudeRunnerEffect::PermissionRequest(
            ClaudePermissionRequest {
                request_id: "request-1".into(),
                tool_use_id: "tool-1".into(),
                tool_name: "Bash".into(),
            }
        )]
    );
    assert!(!format!("{effects:?}").contains("private command"));
    runner
        .respond_permission("run-1", "request-1", ClaudePermissionBehavior::Allow, true)
        .unwrap();
    let writes = state.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    let response: serde_json::Value = serde_json::from_slice(&writes[1]).unwrap();
    assert_eq!(response["type"], "control_response");
    assert_eq!(response["response"]["request_id"], "request-1");
    assert_eq!(
        response["response"]["response"]["updatedPermissions"][0]["path"],
        "/private"
    );
}

#[test]
fn unknown_or_duplicate_permission_requests_never_write_a_response() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    state.output.lock().unwrap().push_back(br#"{"type":"control_request","request_id":"request-1","request":{"subtype":"can_use_tool","tool_use_id":"tool-1","tool_name":"Bash"}}
{"type":"control_request","request_id":"request-1","request":{"subtype":"can_use_tool","tool_use_id":"tool-2","tool_name":"Bash"}}
"#.to_vec());
    let effects = runner.poll("run-1").unwrap().unwrap();
    assert!(matches!(
        effects[0],
        ClaudeRunnerEffect::PermissionRequest(_)
    ));
    assert!(
        matches!(effects[1], ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic { ref classification }
    )) if classification == "duplicateClaudePermissionRequest")
    );
    assert!(
        runner
            .respond_permission("run-1", "unknown", ClaudePermissionBehavior::Deny, false)
            .is_err()
    );
    assert_eq!(state.writes.lock().unwrap().len(), 1);
}

#[test]
fn runner_correlates_native_tool_results_with_the_preceding_tool_start() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    state.output.lock().unwrap().push_back(
        br#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-1","name":"Bash"}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"private output"}]}}
"#
        .to_vec(),
    );

    let effects = runner.poll("run-1").unwrap().unwrap();
    assert!(matches!(
        &effects[0],
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity { activity }
        )) if activity.tool_use_id == "tool-1" && activity.tool_name == "Bash" && activity.phase == ToolPhase::Started
    ));
    assert!(matches!(
        &effects[1],
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity { activity }
        )) if activity.tool_use_id == "tool-1" && activity.tool_name == "Bash" && activity.phase == ToolPhase::Completed && activity.output_digest.as_deref().is_some_and(|digest| digest.starts_with("sha256:"))
    ));
    assert!(!format!("{effects:?}").contains("private output"));
}
