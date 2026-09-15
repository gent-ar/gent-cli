use super::{ClaudeRunnerEffect, OwnedRun, ProviderProcess};
use crate::PublicProvider;
use crate::claude_tool_results;
use crate::public_protocol::{PublicWireFact, claude_protocol, normalize_public_frame};
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, WorkPhase};
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
        run.permissions.cancel(&frame);
        return Vec::new();
    }
    if frame.get("isReplay").and_then(Value::as_bool) == Some(true) {
        return frame
            .get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| run.steers.remove(uuid))
            .map(|message_id| ClaudeRunnerEffect::SteerConsumed { message_id })
            .into_iter()
            .collect();
    }
    if frame.get("type").and_then(Value::as_str) == Some("user") {
        let mut facts = claude_tool_results::results(&mut run.started_tools, &frame)
            .unwrap_or_else(|| normalize_public_frame(PublicProvider::Claude, &frame));
        remember_launches(run, &frame, &mut facts);
        return facts.into_iter().map(ClaudeRunnerEffect::Fact).collect();
    }
    if let Some(facts) = task_facts(run, &frame) {
        return facts.into_iter().map(ClaudeRunnerEffect::Fact).collect();
    }
    let facts = normalize_public_frame(PublicProvider::Claude, &frame);
    claude_tool_results::remember(&facts, &mut run.started_tools);
    facts.into_iter().map(ClaudeRunnerEffect::Fact).collect()
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
    let tool_use_id = frame
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    match frame.get("subtype").and_then(Value::as_str)? {
        "task_notification" => {
            let phase = claude_protocol::task_terminal(frame)?;
            Some(
                run.child_ids
                    .remove(tool_use_id)
                    .map(|child_id| {
                        PublicWireFact::Event(NormalizedProviderEvent::ChildTerminal {
                            child_id,
                            phase,
                        })
                    })
                    .into_iter()
                    .collect(),
            )
        }
        "task_started" | "task_progress" => match run.child_ids.get(tool_use_id) {
            Some(child_id) => Some(vec![PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ChildPhase {
                    child_id: child_id.clone(),
                    phase: WorkPhase::Running,
                },
            )]),
            None => run.started_tools.get(tool_use_id).map(|started| {
                vec![PublicWireFact::Lifecycle(
                    NormalizedLifecycleSignal::ToolActivity {
                        activity: started.clone(),
                    },
                )]
            }),
        },
        _ => None,
    }
}

fn remember_launches<P>(run: &mut OwnedRun<P>, frame: &Value, facts: &mut Vec<PublicWireFact>)
where
    P: ProviderProcess,
{
    for fact in claude_protocol::background_launches(frame) {
        if let PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id,
            parent_tool_use_id,
        }) = &fact
        {
            run.child_ids
                .entry(parent_tool_use_id.clone())
                .or_insert_with(|| child_id.clone());
        }
        facts.push(fact);
    }
}
