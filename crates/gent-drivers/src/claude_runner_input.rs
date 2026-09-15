use super::{ClaudeRunStart, ClaudeRunnerError, MAX_CLAUDE_FRAME_BYTES};
use crate::goal_projection::project_prompt;
use crate::launch_spec::LaunchIntent;

pub(super) fn input_frame(start: &ClaudeRunStart) -> Result<Vec<u8>, ClaudeRunnerError> {
    if start.run_id.trim().is_empty()
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
    let mut value = serde_json::json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null});
    if let LaunchIntent::Resume { session_id } = &start.intent {
        value["session_id"] = serde_json::Value::String(session_id.clone());
    }
    let mut frame = serde_json::to_vec(&value).map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
}

/// Encodes a later user turn for an already-live Claude stream-json session.
pub fn follow_up_input_frame(
    prompt: &str,
    goal: Option<&gent_types::GoalProjection>,
    content: &[serde_json::Value],
) -> Result<Vec<u8>, ClaudeRunnerError> {
    user_frame(prompt, goal, content, None)
}

pub(super) fn steer_uuid(message_id: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("gent-claude-steer\0{message_id}").as_bytes());
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
    uuid: &str,
    prompt: &str,
    content: &[serde_json::Value],
) -> Result<Vec<u8>, ClaudeRunnerError> {
    user_frame(prompt, None, content, Some(uuid))
}

fn user_frame(
    prompt: &str,
    goal: Option<&gent_types::GoalProjection>,
    content: &[serde_json::Value],
    uuid: Option<&str>,
) -> Result<Vec<u8>, ClaudeRunnerError> {
    if prompt.trim().is_empty() || prompt.len() > MAX_CLAUDE_FRAME_BYTES {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    let prompt = project_prompt(prompt, goal, MAX_CLAUDE_FRAME_BYTES)
        .map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    let content = std::iter::once(serde_json::json!({"type":"text","text":prompt}))
        .chain(content.iter().cloned())
        .collect::<Vec<_>>();
    let mut value = serde_json::json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null});
    if let Some(uuid) = uuid {
        value["uuid"] = serde_json::Value::String(uuid.into());
    }
    let mut frame = serde_json::to_vec(&value).map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
}
