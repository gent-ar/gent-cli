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
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
    FrozenConversationContext, GOAL_SCHEMA_VERSION, GoalBinding, GoalProjection, GoalRecord,
    GoalStatus, NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase, TurnPhase,
    WorkPhase,
};

#[derive(Default)]
struct State {
    output: Mutex<VecDeque<Vec<u8>>>,
    writes: Mutex<Vec<Vec<u8>>>,
    launches: Mutex<Vec<ProviderLaunch>>,
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

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
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
        self.0.launches.lock().unwrap().push(launch.clone());
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
        content: Vec::new(),
        turn_options: ClaudeTurnOptions::from_selection(&AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "claude-haiku".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        })
        .unwrap(),
        goal: None,
        fresh_context: None,
        intent: session.map_or(gent_drivers::LaunchIntent::Start, |session_id| {
            gent_drivers::LaunchIntent::Resume {
                session_id: session_id.into(),
            }
        }),
        workspace_root: root.to_path_buf(),
        workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
        mcp_config: None,
    }
}

fn goal() -> GoalProjection {
    GoalProjection::from_active(&GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
        },
        revision: 3,
        status: GoalStatus::Active,
        reason: gent_types::GoalStatusReason::UserSet,
        objective: "Finish the durable task".into(),
        note: None,
        time_used_seconds: 0,
        active_since: Some(1),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 1,
        updated_at: 1,
    })
    .unwrap()
}

#[test]
fn claude_receives_only_the_gent_owned_active_goal_projection() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    let mut request = start("run-1", directory.path(), None);
    request.goal = Some(goal());
    runner.start(request).unwrap();

    let frame: serde_json::Value =
        serde_json::from_slice(&state.writes.lock().unwrap()[0]).unwrap();
    let text = frame["message"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"goalId\":\"goal-1\""));
    assert!(text.contains("\"goalId\":\"goal-1\""));
    assert!(text.contains("Obey Gent permissions"));
}

#[test]
fn locked_claude_runner_writes_one_documented_prompt_and_normalizes_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
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

const BACKGROUND_SUBAGENT: &str = include_str!("../fixtures/claude-background-subagent.jsonl");
const PARENT_TOOL: &str = "toolu_01VUYEGeCHLycLv5neJzkvz6";
const CHILD: &str = "afd78f7cc9d9fd901";

fn replay_background_subagent() -> Vec<PublicWireFact> {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner
        .start(start("run-1", directory.path(), None))
        .unwrap();
    let mut facts = Vec::new();
    for line in BACKGROUND_SUBAGENT.lines() {
        state
            .output
            .lock()
            .unwrap()
            .push_back(format!("{line}\n").into_bytes());
        for effect in runner.poll("run-1").unwrap().unwrap_or_default() {
            if let ClaudeRunnerEffect::Fact(fact) = effect {
                facts.push(fact);
            }
        }
    }
    facts
}

fn position(facts: &[PublicWireFact], matches: impl Fn(&PublicWireFact) -> bool) -> usize {
    let found: Vec<_> = (0..facts.len())
        .filter(|&index| matches(&facts[index]))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one match, found {found:?}"
    );
    found[0]
}

#[test]
fn real_background_subagent_notification_settles_the_child_once_after_the_turn_terminal() {
    let facts = replay_background_subagent();
    let started = position(&facts, |fact| {
        matches!(fact, PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id, parent_tool_use_id
        }) if child_id == CHILD && parent_tool_use_id == PARENT_TOOL)
    });
    let terminal = position(&facts, |fact| {
        matches!(fact, PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal {
            child_id, phase: WorkPhase::Done
        }) if child_id == CHILD)
    });
    let ready: Vec<_> = (0..facts.len())
        .filter(|&index| {
            facts[index]
                == PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
                    phase: TurnPhase::Ready,
                })
        })
        .collect();
    let continuation = position(&facts, |fact| {
        matches!(fact, PublicWireFact::Event(NormalizedProviderEvent::Output {
            text, is_partial: false
        }) if text == "The agent has completed. There are **3 .txt files** in the workspace.")
    });
    assert_eq!(ready.len(), 2);
    assert!(started < ready[0] && ready[0] < terminal);
    assert!(terminal < continuation && continuation < ready[1]);
    assert!(facts.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ChildPhase {
            child_id, phase: WorkPhase::Running
        }) if child_id == CHILD
    )));
    let diagnostics: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification,
            }) => Some(classification.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !diagnostics.iter().any(|classification| {
            classification.contains("Frame")
                || classification.contains("Task")
                || classification.contains("ToolResult")
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn real_background_subagent_tools_and_report_belong_to_the_child_not_the_root_turn() {
    let facts = replay_background_subagent();
    let child_tools: Vec<_> = facts
        .iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity })
                if activity.tool_use_id != PARENT_TOOL =>
            {
                Some((
                    activity.tool_name.as_str(),
                    activity.phase.clone(),
                    activity.parent_tool_use_id.as_deref(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        child_tools,
        [
            ("ToolSearch", ToolPhase::Started, Some(PARENT_TOOL)),
            ("ToolSearch", ToolPhase::Completed, Some(PARENT_TOOL)),
            ("Bash", ToolPhase::Started, Some(PARENT_TOOL)),
            ("Bash", ToolPhase::Completed, Some(PARENT_TOOL)),
        ]
    );
    assert!(facts.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id, text, is_partial: false
        }) if tool_use_id == PARENT_TOOL && text == "3"
    )));
    assert!(!facts.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::Output { text, .. })
            if text == "3" || text.contains("Glob tool")
    )));
}

#[test]
fn claude_background_progress_reuses_the_known_tool_name() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner
        .start(start("run-1", directory.path(), None))
        .unwrap();
    state.output.lock().unwrap().push_back(
        br#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"task-1","name":"Task"}]}}
{"type":"system","subtype":"task_progress","tool_use_id":"task-1"}
"#
        .to_vec(),
    );
    let effects = runner.poll("run-1").unwrap().unwrap();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity { activity }
        )) if activity.tool_use_id == "task-1"
            && activity.tool_name == "Task"
            && activity.phase == ToolPhase::Started
    )));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic { classification }
        )) if classification == "unresolvedClaudeBackgroundTask"
    )));
}

#[test]
fn locked_claude_runner_launches_only_the_durable_model_and_bounded_plan_mode() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    let mut request = start("run-1", directory.path(), None);
    request.turn_options = ClaudeTurnOptions::from_selection(&AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "claude-sonnet".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Plan,
    })
    .unwrap();
    runner.start(request).unwrap();

    assert_eq!(
        state.launches.lock().unwrap()[0].workspace_root,
        Some(directory.path().into())
    );
    let arguments = &state.launches.lock().unwrap()[0].arguments;
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--model", "claude-sonnet"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--permission-prompt-tool", "stdio"])
    );
    assert!(
        !arguments
            .iter()
            .any(|value| value == "auto" || value == "bypassPermissions")
    );
}

#[test]
fn locked_claude_runner_injects_the_authoritative_mcp_config() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    let config = directory.path().join("mcp.json");
    std::fs::write(&config, "{}").unwrap();
    let mut request = start("run-1", directory.path(), None);
    request.mcp_config = Some(config.clone());
    runner.start(request).unwrap();
    assert!(
        state.launches.lock().unwrap()[0]
            .arguments
            .windows(2)
            .any(|pair| pair == ["--mcp-config", config.to_string_lossy().as_ref()])
    );
}

#[test]
fn resume_binds_the_prompt_and_exit_drains_before_settlement() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
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

#[test]
fn fresh_gent_context_never_reuses_a_claude_native_session() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    let mut request = start("run-1", directory.path(), Some("private-session"));
    request.fresh_context = Some(FrozenConversationContext::cleared(AgentChatConversationId(
        "conversation-1".into(),
    )));
    assert!(runner.start(request).is_err());
    assert!(state.writes.lock().unwrap().is_empty());
}

const MISSING_SESSION_RESULT: &[u8] =
    include_bytes!("../fixtures/claude-resume-session-missing.jsonl");
const MOVED_CWD_RESUME: &[u8] = include_bytes!("../fixtures/claude-resume-moved-cwd.jsonl");

fn resumed_effects(first_frames: &[&[u8]], session: Option<&str>) -> Vec<ClaudeRunnerEffect> {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(8, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    runner
        .start(start("run-1", directory.path(), session))
        .unwrap();
    for frame in first_frames {
        state.output.lock().unwrap().push_back(frame.to_vec());
    }
    let mut effects = Vec::new();
    while let Some(batch) = runner.poll("run-1").unwrap() {
        effects.extend(batch);
    }
    effects
}

fn failure_facts(effects: &[ClaudeRunnerEffect]) -> usize {
    effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                ClaudeRunnerEffect::Fact(PublicWireFact::Event(
                    NormalizedProviderEvent::ProviderFailure { .. }
                )) | ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::RootPhase {
                        phase: TurnPhase::Failed
                    }
                ))
            )
        })
        .count()
}

#[test]
fn a_resumed_session_the_provider_no_longer_has_is_reported_as_unavailable_not_failed() {
    let effects = resumed_effects(&[MISSING_SESSION_RESULT], Some("private-session"));
    assert_eq!(effects, [ClaudeRunnerEffect::ResumeUnavailable]);
}

#[test]
fn a_session_resumed_from_a_moved_workspace_still_resumes_as_recorded_from_claude() {
    let effects = resumed_effects(&[MOVED_CWD_RESUME], Some("session-moved"));
    assert!(!effects.contains(&ClaudeRunnerEffect::ResumeUnavailable));
    assert_eq!(failure_facts(&effects), 0);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready
            }
        ))
    )));
}

#[test]
fn the_same_error_result_on_a_fresh_launch_stays_a_provider_failure() {
    let effects = resumed_effects(&[MISSING_SESSION_RESULT], None);
    assert!(!effects.contains(&ClaudeRunnerEffect::ResumeUnavailable));
    assert!(failure_facts(&effects) > 0);
}

#[test]
fn a_resumed_session_that_initialized_before_failing_is_a_genuine_failure() {
    let effects = resumed_effects(
        &[
            br#"{"type":"system","subtype":"init","session_id":"private-session"}
"#,
            MISSING_SESSION_RESULT,
        ],
        Some("private-session"),
    );
    assert!(!effects.contains(&ClaudeRunnerEffect::ResumeUnavailable));
    assert!(failure_facts(&effects) > 0);
}

#[test]
fn a_resumed_error_that_spent_api_time_or_turns_is_a_genuine_failure() {
    for frame in [
        br#"{"type":"result","subtype":"error_during_execution","duration_api_ms":812,"is_error":true,"num_turns":0,"session_id":"private-session","errors":["API Error: 401"]}
"#
        .as_slice(),
        br#"{"type":"result","subtype":"error_during_execution","duration_api_ms":0,"is_error":true,"num_turns":1,"session_id":"private-session"}
"#
        .as_slice(),
        br#"{"type":"result","subtype":"error_max_turns","duration_api_ms":0,"is_error":true,"num_turns":0,"session_id":"private-session"}
"#
        .as_slice(),
    ] {
        let effects = resumed_effects(&[frame], Some("private-session"));
        assert!(!effects.contains(&ClaudeRunnerEffect::ResumeUnavailable));
        assert!(failure_facts(&effects) > 0);
    }
}

#[test]
fn recreating_a_lost_session_reuses_its_identity_with_gent_history_and_no_resume() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = ClaudeStreamRunner::new(
        Launcher(Arc::clone(&state)),
        BufferPolicy::new(1, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    );
    let mut request = start("run-1", directory.path(), None);
    request.intent = gent_drivers::LaunchIntent::Recreate {
        session_id: "private-session".into(),
    };
    request.fresh_context = Some(FrozenConversationContext::cleared(AgentChatConversationId(
        "conversation-1".into(),
    )));
    runner.start(request).unwrap();

    let launch = state.launches.lock().unwrap()[0].clone();
    assert!(
        launch
            .arguments
            .windows(2)
            .any(|pair| pair == ["--session-id", "private-session"])
    );
    assert!(
        !launch
            .arguments
            .iter()
            .any(|argument| argument == "--resume")
    );
    let input: serde_json::Value =
        serde_json::from_slice(&state.writes.lock().unwrap()[0]).unwrap();
    assert!(input.get("session_id").is_none());
    state
        .output
        .lock()
        .unwrap()
        .push_back(MISSING_SESSION_RESULT.to_vec());
    let effects = runner.poll("run-1").unwrap().unwrap();
    assert!(!effects.contains(&ClaudeRunnerEffect::ResumeUnavailable));
}

fn malformed_tolerance_runner(state: &Arc<State>) -> ClaudeStreamRunner<Launcher, Process> {
    ClaudeStreamRunner::new(
        Launcher(Arc::clone(state)),
        BufferPolicy::new(4, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
    )
}

fn transport_diagnostic(classification: &str) -> ClaudeRunnerEffect {
    ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))
}

#[test]
fn an_unparseable_stdout_line_is_a_typed_diagnostic_that_never_poisons_the_next_frame() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = malformed_tolerance_runner(&state);
    runner
        .start(start("run-1", directory.path(), None))
        .unwrap();
    state.output.lock().unwrap().push_back(
        br#"{"type":"system","subtype":"init","session_id":"private-session"}
not json at all
{"type":"assistant","message":{"content":[{"type":"text"}]}}
{"type":"future-frame-kind"}
{"type":"assistant","message":{"content":[{"type":"text","text":"still parsing"}]}}
"#
        .to_vec(),
    );
    let effects = runner.poll("run-1").unwrap().unwrap();
    for classification in [
        "malformedClaudeFrame",
        "malformedClaudeText",
        "unsupportedClaudeFrame",
    ] {
        assert!(
            effects.contains(&transport_diagnostic(classification)),
            "{classification} missing from {effects:?}"
        );
    }
    assert!(effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: false
        })) if text == "still parsing"
    )));
}

#[test]
fn an_over_ceiling_claude_frame_is_skipped_with_a_typed_diagnostic_and_the_stream_continues() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State::default());
    let mut runner = malformed_tolerance_runner(&state);
    runner
        .start(start("run-1", directory.path(), None))
        .unwrap();
    {
        let mut output = state.output.lock().unwrap();
        for _ in 0..=gent_drivers::MAX_PROVIDER_FRAME_BYTES / 4096 {
            output.push_back(vec![b'x'; 4096]);
        }
        output.push_back(
            b"\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"after\"}]}}\n"
                .to_vec(),
        );
    }
    let mut effects = Vec::new();
    while !state.output.lock().unwrap().is_empty() {
        effects.extend(runner.poll("run-1").unwrap().unwrap_or_default());
    }
    assert!(effects.contains(&transport_diagnostic(
        gent_types::OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        ClaudeRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: false
        })) if text == "after"
    )));
}
