use super::{ClaudeRunnerEffect, OwnedRun, ProviderProcess};
use crate::PublicProvider;
use crate::claude_tool_results;
use crate::public_protocol::{PublicWireFact, claude_protocol, normalize_public_frame};
use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase, WorkPhase,
};
use serde_json::Value;

pub(super) fn normalize<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
    raw: &[u8],
) -> Vec<ClaudeRunnerEffect> {
    let frame: Value = match serde_json::from_slice(raw) {
        Ok(frame) => frame,
        Err(_) => return diagnostic("malformedClaudeFrame"),
    };
    if std::mem::take(&mut run.awaiting_resumed_frame) && resumed_session_unavailable(&frame) {
        return vec![ClaudeRunnerEffect::ResumeUnavailable];
    }
    if frame.get("type").and_then(Value::as_str) == Some("control_request") {
        return match run.permissions.accept(&frame) {
            Ok(request) if run.plans_for_review && request.tool_name == "ExitPlanMode" => {
                submit_plan_for_review(run, &request.request_id)
            }
            Ok(request) => vec![ClaudeRunnerEffect::PermissionRequest(request)],
            Err(classification) => reject_control_request(run, &frame, classification),
        };
    }
    if frame.get("type").and_then(Value::as_str) == Some("control_cancel_request") {
        return run
            .permissions
            .cancel(&frame)
            .map(|request_id| ClaudeRunnerEffect::PermissionCancelled { request_id })
            .into_iter()
            .collect();
    }
    if frame.get("isReplay").and_then(Value::as_bool) == Some(true) {
        return run.messages.lifecycle(frame.get("uuid"), "started");
    }
    if frame.get("type").and_then(Value::as_str) == Some("command_lifecycle") {
        let state = frame
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return run.messages.lifecycle(frame.get("command_uuid"), state);
    }
    if let Some(idle) = session_state(&frame) {
        return vec![ClaudeRunnerEffect::SessionState {
            idle: idle && run.background_commands.is_empty(),
        }];
    }
    if frame.get("type").and_then(Value::as_str) == Some("user") {
        let facts = claude_tool_results::results(&mut run.started_tools, &frame)
            .unwrap_or_else(|| normalize_public_frame(PublicProvider::Claude, &frame));
        return facts
            .into_iter()
            .map(|fact| detached_launch(run, fact))
            .map(ClaudeRunnerEffect::Fact)
            .collect();
    }
    if let Some(facts) = task_facts(run, &frame) {
        return facts.into_iter().map(ClaudeRunnerEffect::Fact).collect();
    }
    let mut effects = Vec::new();
    let mut facts = normalize_public_frame(PublicProvider::Claude, &frame);
    claude_tool_results::remember(&facts, &mut run.started_tools);
    run.authentication_failed |= claude_protocol::authentication_failed(&frame);
    run.rate_limited |= claude_protocol::rate_limited(&frame);
    if let Some(resets_at) = claude_protocol::rate_limit_resets_at(&frame) {
        run.rate_limit_resets_at = Some(resets_at);
    }
    if frame.get("type").and_then(Value::as_str) == Some("result") {
        effects.push(run.messages.result(&frame));
        let rate_limited = std::mem::take(&mut run.rate_limited);
        if std::mem::take(&mut run.authentication_failed) {
            facts = facts
                .into_iter()
                .map(claude_protocol::sign_in_required)
                .collect();
        } else if rate_limited {
            facts = facts
                .into_iter()
                .map(|fact| claude_protocol::rate_limit_reached(fact, run.rate_limit_resets_at))
                .collect();
        }
    }
    effects.extend(facts.into_iter().map(ClaudeRunnerEffect::Fact));
    effects.extend(run.messages.turn_started(&frame));
    effects
}

fn session_state(frame: &Value) -> Option<bool> {
    (frame.get("type").and_then(Value::as_str) == Some("system")
        && frame.get("subtype").and_then(Value::as_str) == Some("session_state_changed"))
    .then(|| frame.get("state").and_then(Value::as_str) == Some("idle"))
}

fn resumed_session_unavailable(frame: &Value) -> bool {
    frame.get("type").and_then(Value::as_str) == Some("result")
        && frame.get("subtype").and_then(Value::as_str) == Some("error_during_execution")
        && frame.get("is_error").and_then(Value::as_bool) == Some(true)
        && frame.get("num_turns").and_then(Value::as_u64) == Some(0)
        && frame.get("duration_api_ms").and_then(Value::as_u64) == Some(0)
}

fn submit_plan_for_review<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
    request_id: &str,
) -> Vec<ClaudeRunnerEffect> {
    run.permissions.settle(request_id);
    match run
        .process
        .write_frame(&crate::claude_control::encode_plan_review_response(
            request_id,
        )) {
        Ok(()) => Vec::new(),
        Err(_) => diagnostic("claudePlanReviewResponseUnwritten"),
    }
}

fn reject_control_request<P: ProviderProcess>(
    run: &mut OwnedRun<P>,
    frame: &Value,
    classification: &'static str,
) -> Vec<ClaudeRunnerEffect> {
    let mut effects = diagnostic(classification);
    let request_id = crate::claude_control::control_request_id(frame)
        .filter(|_| classification != "duplicateClaudePermissionRequest");
    if let Some(request_id) = request_id {
        let response = crate::claude_control::encode_control_error(request_id, classification);
        if run.process.write_frame(&response).is_err() {
            effects.extend(diagnostic("claudeControlErrorResponseUnwritten"));
        }
    }
    effects
}

fn diagnostic(classification: &str) -> Vec<ClaudeRunnerEffect> {
    vec![ClaudeRunnerEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}

fn task_facts<P>(run: &mut OwnedRun<P>, frame: &Value) -> Option<Vec<PublicWireFact>>
where
    P: ProviderProcess,
{
    if frame.get("type").and_then(Value::as_str) != Some("system") {
        return None;
    }
    let task_id = frame
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let tool_use_id = frame
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    match frame.get("subtype").and_then(Value::as_str)? {
        "task_notification" | "task_updated" => {
            let phase = claude_protocol::task_terminal(frame)?;
            if let Some(started) = run.background_commands.remove(task_id) {
                return Some(vec![tool_phase(
                    started,
                    claude_protocol::command_terminal(&phase),
                )]);
            }
            Some(
                run.children
                    .remove(task_id)
                    .then(|| {
                        PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal {
                            child_id: task_id.into(),
                            phase,
                        })
                    })
                    .into_iter()
                    .collect(),
            )
        }
        "task_started" if claude_protocol::backgrounded_agent(frame) => {
            run.children.insert(task_id.into());
            Some(vec![PublicWireFact::Event(
                NormalizedProviderEvent::ChildStarted {
                    child_id: task_id.into(),
                    parent_tool_use_id: tool_use_id?.into(),
                },
            )])
        }
        "task_started" if claude_protocol::backgrounded_command(frame) => {
            let started = run.started_tools.get(tool_use_id?)?.clone();
            run.background_commands
                .insert(task_id.into(), started.clone());
            Some(vec![tool_phase(started, ToolPhase::Background)])
        }
        "task_progress" if run.children.contains(task_id) => Some(vec![PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ChildPhase {
                child_id: task_id.into(),
                phase: WorkPhase::Running,
            },
        )]),
        "task_started" | "task_progress" => run.started_tools.get(tool_use_id?).map(|started| {
            vec![PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity {
                    activity: started.clone(),
                },
            )]
        }),
        _ => None,
    }
}

fn tool_phase(activity: ToolActivity, phase: ToolPhase) -> PublicWireFact {
    PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity { phase, ..activity },
    })
}

fn detached_launch<P>(run: &OwnedRun<P>, fact: PublicWireFact) -> PublicWireFact {
    match fact {
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity })
            if activity.phase == ToolPhase::Completed
                && run
                    .background_commands
                    .values()
                    .any(|command| command.tool_use_id == activity.tool_use_id) =>
        {
            tool_phase(activity, ToolPhase::Background)
        }
        fact => fact,
    }
}

pub(super) fn drain<P: ProviderProcess>(run: &mut OwnedRun<P>) -> Option<Vec<ClaudeRunnerEffect>> {
    let mut effects = Vec::new();
    while run.output.queued_frames() > 0 {
        let (frame, _) = run.output.take_frame();
        let Some(frame) = frame else { break };
        effects.extend(normalize(run, &frame));
    }
    effects.extend(
        crate::public_protocol::oversized_frames_skipped(run.output.take_skipped_frames())
            .map(ClaudeRunnerEffect::Fact),
    );
    (!effects.is_empty()).then_some(effects)
}
