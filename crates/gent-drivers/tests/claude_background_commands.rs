use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::claude_runner::{ClaudeRunStart, ClaudeRunnerEffect, ClaudeStreamRunner};
use gent_drivers::claude_turn_options::ClaudeTurnOptions;
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::lock::capture;
use gent_drivers::public_protocol::PublicWireFact;
use gent_drivers::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    NormalizedLifecycleSignal, SandboxWorkspaceAccess, ToolPhase,
};

const LAUNCH: &[u8] = br#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"PowerShell","input":{"command":"py -3 -c \"import time; time.sleep(60)\"","run_in_background":true}}]}}
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","task_type":"local_bash","description":"Sleep"}]}
{"type":"system","subtype":"task_started","task_id":"b1","tool_use_id":"toolu_1","description":"Sleep","is_backgrounded":true,"task_type":"local_bash"}
{"type":"user","message":{"content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"Command running in background with ID: b1.","is_error":false}]},"parent_tool_use_id":null}
"#;

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

fn runner(root: &Path, state: &Arc<State>) -> ClaudeStreamRunner<Launcher, Process> {
    let executable = root.join("claude");
    std::fs::write(&executable, "test executable").unwrap();
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(state)),
        BufferPolicy::new(2, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner
        .start(ClaudeRunStart {
            run_id: "run-1".into(),
            message_id: "message-1".into(),
            lock: capture("claude", &executable, "2.1.0", "entry").unwrap(),
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
            intent: gent_drivers::LaunchIntent::Start,
            workspace_root: root.to_path_buf(),
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
            mcp_config: None,
        })
        .unwrap();
    runner
}

fn phases(
    runner: &mut ClaudeStreamRunner<Launcher, Process>,
    frames: &[u8],
    state: &State,
) -> Vec<ToolPhase> {
    state.output.lock().unwrap().push_back(frames.to_vec());
    runner
        .poll("run-1")
        .unwrap()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|effect| match effect {
            ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity { activity },
            )) if activity.tool_use_id == "toolu_1" => Some(activity.phase),
            _ => None,
        })
        .collect()
}

fn written(state: &State) -> Vec<serde_json::Value> {
    state
        .writes
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_slice(frame).unwrap())
        .filter(|frame: &serde_json::Value| frame["type"] == "control_request")
        .map(|frame| frame["request"].clone())
        .collect()
}

#[test]
fn a_background_shell_stays_live_past_its_launch_until_claude_reports_its_end() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    assert_eq!(
        phases(&mut runner, LAUNCH, &state),
        [
            ToolPhase::Started,
            ToolPhase::Background,
            ToolPhase::Background
        ]
    );
    assert_eq!(
        phases(
            &mut runner,
            br#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}
{"type":"system","subtype":"task_updated","task_id":"b1","patch":{"status":"completed","end_time":1}}
{"type":"system","subtype":"task_notification","task_id":"b1","tool_use_id":"toolu_1","status":"completed","output_file":"","summary":"done"}
"#,
            &state,
        ),
        [ToolPhase::Completed]
    );
}

fn session_states(
    runner: &mut ClaudeStreamRunner<Launcher, Process>,
    frames: &[u8],
    state: &State,
) -> Vec<bool> {
    state.output.lock().unwrap().push_back(frames.to_vec());
    runner
        .poll("run-1")
        .unwrap()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|effect| match effect {
            ClaudeRunnerEffect::SessionState { idle } => Some(idle),
            _ => None,
        })
        .collect()
}

#[test]
fn claude_idle_with_a_background_shell_open_keeps_the_session_running_until_its_end() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    phases(&mut runner, LAUNCH, &state);
    assert_eq!(
        session_states(
            &mut runner,
            br#"{"type":"result","subtype":"success","is_error":false,"result":"LAUNCHED"}
{"type":"system","subtype":"session_state_changed","state":"idle"}
"#,
            &state,
        ),
        [false]
    );
    assert_eq!(
        session_states(
            &mut runner,
            br#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}
{"type":"system","subtype":"task_updated","task_id":"b1","patch":{"status":"completed","end_time":1}}
{"type":"system","subtype":"task_notification","task_id":"b1","tool_use_id":"toolu_1","status":"completed","output_file":"","summary":"done"}
{"type":"system","subtype":"session_state_changed","state":"running"}
{"type":"result","subtype":"success","is_error":false,"result":"DONE-SEEN"}
{"type":"system","subtype":"session_state_changed","state":"idle"}
"#,
            &state,
        ),
        [false, true]
    );
}

#[test]
fn stop_ends_every_background_shell_through_claude_stop_task() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = runner(directory.path(), &state);
    phases(&mut runner, LAUNCH, &state);
    runner.interrupt("run-1").unwrap();
    assert_eq!(
        written(&state),
        [
            serde_json::json!({"subtype":"interrupt","cancel_queued":true}),
            serde_json::json!({"subtype":"stop_task","task_id":"b1"}),
        ]
    );
    assert_eq!(
        phases(
            &mut runner,
            br#"{"type":"system","subtype":"task_updated","task_id":"b1","patch":{"status":"killed","end_time":1}}
{"type":"system","subtype":"task_notification","task_id":"b1","tool_use_id":"toolu_1","status":"stopped","output_file":"","summary":"Sleep"}
{"type":"control_response","response":{"subtype":"success","request_id":"gent-stop-task-b1","response":{}}}
"#,
            &state,
        ),
        [ToolPhase::Interrupted]
    );
    runner.interrupt("run-1").unwrap();
    assert_eq!(written(&state).len(), 3);
}
