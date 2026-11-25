use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use base64::Engine;
use gent_drivers::conversation_context_input::{
    HistoryWindow, MAX_FRESH_CONTEXT_INPUT_BYTES, render_fresh_conversation_input,
};
use gent_drivers::goal_projection::project_prompt;
use gent_ports::{
    ClaurstDrainBatch, ClaurstDrainRequest, ClaurstNormalizedFact, ClaurstPermissionReply,
    ClaurstPermissionRequest, ClaurstPromptAttachment, ClaurstSessionBinding, ClaurstStartRequest,
    ClaurstSubmitRequest, PortError,
};

use crate::claurst_acp_transport::{ClaurstAcpStdio, ClaurstAcpTransportError};

use super::support::{checkpoint, invalid, project, project_terminal, provider, unavailable};
use super::{BridgeState, RunSession, SourceState};

pub(super) fn start_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    workspace: PathBuf,
    request: ClaurstStartRequest,
) -> Result<ClaurstSessionBinding, PortError> {
    request.validate().map_err(|_| unsent("start request"))?;
    let mut state = state.lock().map_err(|_| unsent("ACP bridge lock"))?;
    if state.sources.contains_key(&request.source_id) {
        return Err(unsent("duplicate source"));
    }
    let rendered = render_fresh_conversation_input(
        &request.context,
        &request.prompt,
        state.history_input_bytes,
    )
    .map_err(|_| unsent("frozen conversation context"))?;
    let summary_coverage = request
        .context
        .summary
        .as_ref()
        .map(|summary| summary.covers_through_ordinal);
    let continued = state
        .run_sessions
        .get(&request.run_id)
        .filter(|session| {
            session.summary_coverage == summary_coverage
                && rendered.window() != HistoryWindow::Truncated
        })
        .map(|session| session.session_id.clone());
    let input = match continued {
        Some(_) => request.prompt.clone(),
        None => rendered.prompt().to_owned(),
    };
    let prompt = project_prompt(
        &input,
        request.goal.as_ref().map(|goal| &goal.goal),
        state.history_input_bytes,
    )
    .map_err(|_| unsent("active goal context"))?;
    let session_id = match continued {
        Some(session_id) => session_id,
        None => state
            .transport
            .initialize_session(&workspace)
            .map_err(prompt_unsent)?,
    };
    state.run_sessions.insert(
        request.run_id.clone(),
        RunSession {
            session_id: session_id.clone(),
            summary_coverage,
        },
    );
    let content = prompt_content(
        &prompt,
        &request.attachments,
        state.transport.supports_images(),
    )
    .map_err(unsent)?;
    state
        .transport
        .prompt_content(&session_id, content)
        .map_err(|error| match error {
            ClaurstAcpTransportError::Io(_) => provider(error),
            _ => prompt_unsent(error),
        })?;
    let binding = ClaurstSessionBinding {
        run_id: request.run_id,
        source_id: request.source_id,
        opaque_session_id: session_id,
    };
    state.sources.insert(
        binding.source_id.clone(),
        SourceState {
            binding: binding.clone(),
            cursor: 0,
            terminal: false,
        },
    );
    Ok(binding)
}

pub(super) fn bind_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    binding: ClaurstSessionBinding,
) -> Result<(), PortError> {
    let state = state.lock().map_err(|_| unavailable("ACP bridge lock"))?;
    state
        .sources
        .get(&binding.source_id)
        .is_some_and(|source| source.binding == binding)
        .then_some(())
        .ok_or_else(|| invalid("unknown session binding"))
}

pub(super) fn cancel_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    binding: ClaurstSessionBinding,
) -> Result<(), PortError> {
    let mut state = state.lock().map_err(|_| unavailable("ACP bridge lock"))?;
    let source = state
        .sources
        .get(&binding.source_id)
        .ok_or_else(|| invalid("unknown source"))?;
    let run_sessions: std::collections::BTreeSet<_> = state
        .sources
        .values()
        .filter(|source| source.binding.run_id == binding.run_id)
        .map(|source| source.binding.opaque_session_id.clone())
        .collect();
    let mut sessions: std::collections::BTreeSet<_> = state
        .transport
        .background_sessions()
        .intersection(&run_sessions)
        .cloned()
        .collect();
    if !source.terminal {
        sessions.insert(binding.opaque_session_id.clone());
    }
    if source.binding != binding || sessions.is_empty() {
        return Err(invalid("inactive session binding"));
    }
    state.run_sessions.remove(&binding.run_id);
    for session in sessions {
        state.transport.cancel(&session).map_err(provider)?;
    }
    Ok(())
}

pub(super) fn submit_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    request: ClaurstSubmitRequest,
) -> Result<(), PortError> {
    request.validate().map_err(|_| invalid("submit request"))?;
    let mut state = state.lock().map_err(|_| unavailable("ACP bridge lock"))?;
    let source = state
        .sources
        .get(&request.binding.source_id)
        .ok_or_else(|| invalid("unknown source"))?;
    if source.binding != request.binding || source.terminal {
        return Err(invalid("inactive session binding"));
    }
    let prompt = project_prompt(
        &request.prompt,
        request.goal.as_ref().map(|goal| &goal.goal),
        MAX_FRESH_CONTEXT_INPUT_BYTES,
    )
    .map_err(|_| invalid("active goal context"))?;
    let content = prompt_content(
        &prompt,
        &request.attachments,
        state.transport.supports_images(),
    )
    .map_err(invalid)?;
    state
        .transport
        .prompt_content(&request.binding.opaque_session_id, content)
        .map_err(provider)
}

fn prompt_content(
    prompt: &str,
    attachments: &[ClaurstPromptAttachment],
    supports_images: bool,
) -> Result<Vec<serde_json::Value>, &'static str> {
    let mut content = vec![serde_json::json!({"type":"text","text":prompt})];
    for attachment in attachments {
        if !supports_images || !attachment.media_type.starts_with("image/") {
            return Err("Claurst ACP cannot accept this attachment");
        }
        content.push(serde_json::json!({
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(&attachment.bytes),
            "mimeType": attachment.media_type,
        }));
    }
    Ok(content)
}

pub(super) fn drain_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    request: ClaurstDrainRequest,
) -> Result<ClaurstDrainBatch, PortError> {
    if !request.is_bounded() {
        return Err(invalid("unbounded drain"));
    }
    let mut state = state.lock().map_err(|_| unavailable("ACP bridge lock"))?;
    let source = state
        .sources
        .get(&request.source_id)
        .ok_or_else(|| invalid("unknown source"))?;
    let follows_background = state.transport.has_background_commands()
        && state.sources.values().all(|source| source.terminal);
    if source.binding.run_id != request.run_id
        || source.cursor != request.after_cursor
        || (source.terminal && !follows_background)
    {
        return Err(invalid("stale or terminal drain"));
    }
    let acp = state.transport.drain(request.limit).map_err(provider)?;
    let source = state
        .sources
        .get_mut(&request.source_id)
        .expect("source remains while bridge lock is held");
    let facts = acp
        .facts
        .into_iter()
        .map(|fact| {
            source.cursor += 1;
            ClaurstNormalizedFact {
                source_id: request.source_id.clone(),
                cursor: source.cursor,
                value: project(fact),
            }
        })
        .collect();
    let terminal = acp.terminal.map(project_terminal);
    source.terminal |= terminal.is_some();
    let ended_run = terminal
        .as_ref()
        .is_some_and(|terminal| *terminal != gent_ports::ClaurstTerminal::Completed)
        .then(|| source.binding.run_id.clone());
    let batch = ClaurstDrainBatch {
        facts,
        permissions: acp
            .permissions
            .into_iter()
            .map(|permission| ClaurstPermissionRequest {
                request_id: permission.request_id,
                tool_use_id: permission.tool_use_id,
                tool_name: permission.tool_name,
                category: permission.category,
                input: permission.input,
                child_id: permission.child_id,
            })
            .collect(),
        checkpoint: Some(checkpoint(&source.binding, source.cursor)),
        session_binding: Some(source.binding.clone()),
        terminal,
    };
    if let Some(run_id) = ended_run {
        state.run_sessions.remove(&run_id);
    }
    Ok(batch)
}

pub(super) fn respond_permission_blocking<S: ClaurstAcpStdio>(
    state: Arc<Mutex<BridgeState<S>>>,
    binding: ClaurstSessionBinding,
    request_id: &str,
    reply: ClaurstPermissionReply,
) -> Result<(), PortError> {
    let mut state = state.lock().map_err(|_| unavailable("ACP bridge lock"))?;
    state
        .sources
        .get(&binding.source_id)
        .is_some_and(|source| source.binding == binding)
        .then_some(())
        .ok_or_else(|| PortError::NotPending("inactive Claurst permission source".into()))?;
    state
        .transport
        .respond_permission(request_id, reply)
        .map_err(provider)
}

fn unsent(what: &str) -> PortError {
    PortError::NotDelivered(format!("invalid Claurst ACP {what}"))
}

fn prompt_unsent(error: ClaurstAcpTransportError) -> PortError {
    PortError::NotDelivered(error.to_string())
}
