use super::{ClaudeRunStart, ClaudeRunnerError, MAX_CLAUDE_FRAME_BYTES};
use crate::goal_projection::project_prompt;

pub(super) fn input_frame(start: &ClaudeRunStart) -> Result<Vec<u8>, ClaudeRunnerError> {
    if start.run_id.trim().is_empty()
        || start.prompt.trim().is_empty()
        || start.prompt.len() > MAX_CLAUDE_FRAME_BYTES
        || (start.fresh_context.is_some() && start.resume_session_id.is_some())
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
    if let Some(session_id) = &start.resume_session_id {
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
    if prompt.trim().is_empty() || prompt.len() > MAX_CLAUDE_FRAME_BYTES {
        return Err(ClaudeRunnerError::InvalidPrompt);
    }
    let prompt = project_prompt(prompt, goal, MAX_CLAUDE_FRAME_BYTES)
        .map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    let content = std::iter::once(serde_json::json!({"type":"text","text":prompt}))
        .chain(content.iter().cloned())
        .collect::<Vec<_>>();
    let mut frame = serde_json::to_vec(
        &serde_json::json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null}),
    )
    .map_err(|_| ClaudeRunnerError::InvalidPrompt)?;
    frame.push(b'\n');
    Ok(frame)
}
