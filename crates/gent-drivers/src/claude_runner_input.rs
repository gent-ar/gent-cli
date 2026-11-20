use super::{ClaudeRunStart, ClaudeRunnerError, MAX_CLAUDE_FRAME_BYTES};
use crate::goal_projection::project_prompt;
use crate::launch_spec::LaunchIntent;

pub(super) fn input_frame(start: &ClaudeRunStart) -> Result<Vec<u8>, ClaudeRunnerError> {
    if start.run_id.trim().is_empty()
        || start.message_id.trim().is_empty()
        || start.prompt.trim().is_empty()
        || start.prompt.len() > MAX_CLAUDE_FRAME_BYTES
        || (start.fresh_context.is_some() && matches!(start.intent, LaunchIntent::Resume { .. }))
    {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    let prompt = match &start.fresh_context {
        Some(context) => crate::conversation_context_input::render_fresh_conversation_input(
            context,
            &start.prompt,
            MAX_CLAUDE_FRAME_BYTES,
        )
        .map_err(|_| ClaudeRunnerError::InvalidPrompt)?
        .prompt()
        .to_owned(),
        None => project_prompt(&start.prompt, start.goal.as_ref(), MAX_CLAUDE_FRAME_BYTES)
            .map_err(|_| ClaudeRunnerError::InvalidPrompt)?,
    };
    let prompt = start
        .fresh_context
        .as_ref()
        .map_or(Ok(prompt.clone()), |_| {
            project_prompt(&prompt, start.goal.as_ref(), MAX_CLAUDE_FRAME_BYTES)
                .map_err(|_| ClaudeRunnerError::InvalidPrompt)
        })?;
    let content = std::iter::once(serde_json::json!({"type":"text","text":prompt}))
        .chain(start.content.clone())
        .collect::<Vec<_>>();
    let mut value = serde_json::json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null,"uuid":message_uuid(&start.message_id)});
    if let LaunchIntent::Resume { session_id } = &start.intent {
        value["session_id"] = serde_json::Value::String(session_id.clone());
    }
    let mut frame = serde_json::to_vec(&value).map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
}

pub(super) fn follow_up_input_frame(
    message_id: &str,
    prompt: &str,
    goal: Option<&gent_types::GoalProjection>,
    content: &[serde_json::Value],
) -> Result<Vec<u8>, ClaudeRunnerError> {
    user_frame(prompt, goal, content, &message_uuid(message_id), None)
}

pub(super) fn resumed_first_input_frame(
    message_id: &str,
    prompt: &str,
    goal: Option<&gent_types::GoalProjection>,
    content: &[serde_json::Value],
    session_id: &str,
) -> Result<Vec<u8>, ClaudeRunnerError> {
    user_frame(
        prompt,
        goal,
        content,
        &message_uuid(message_id),
        Some(session_id),
    )
}

pub(super) fn message_uuid(message_id: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("gent-claude-message\0{message_id}").as_bytes());
    let hex = format!("{digest:x}");
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

pub(super) fn steer_input_frame(
    message_id: &str,
    prompt: &str,
    content: &[serde_json::Value],
) -> Result<Vec<u8>, ClaudeRunnerError> {
    user_frame(prompt, None, content, &message_uuid(message_id), None)
}

fn user_frame(
    prompt: &str,
    goal: Option<&gent_types::GoalProjection>,
    content: &[serde_json::Value],
    uuid: &str,
    session_id: Option<&str>,
) -> Result<Vec<u8>, ClaudeRunnerError> {
    if prompt.trim().is_empty() || prompt.len() > MAX_CLAUDE_FRAME_BYTES {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    let prompt = project_prompt(prompt, goal, MAX_CLAUDE_FRAME_BYTES)
        .map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    let content = std::iter::once(serde_json::json!({"type":"text","text":prompt}))
        .chain(content.iter().cloned())
        .collect::<Vec<_>>();
    let mut value = serde_json::json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null,"uuid":uuid});
    if let Some(session_id) = session_id {
        value["session_id"] = serde_json::Value::String(session_id.into());
    }
    let mut frame = serde_json::to_vec(&value).map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
}

pub(super) fn interrupt_frame(request_id: &str) -> Vec<u8> {
    control_frame(
        request_id,
        serde_json::json!({"subtype": "interrupt", "cancel_queued": true}),
    )
}

pub(super) fn withdraw_frame(message_uuid: &str) -> Vec<u8> {
    control_frame(
        &format!("gent-withdraw-{message_uuid}"),
        serde_json::json!({"subtype": "cancel_async_message", "message_uuid": message_uuid}),
    )
}

pub(super) fn stop_task_frame(task_id: &str) -> Vec<u8> {
    control_frame(
        &format!("gent-stop-task-{task_id}"),
        serde_json::json!({"subtype": "stop_task", "task_id": task_id}),
    )
}

fn control_frame(request_id: &str, request: serde_json::Value) -> Vec<u8> {
    let mut frame = serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
    .to_string()
    .into_bytes();
    frame.push(b'\n');
    frame
}
