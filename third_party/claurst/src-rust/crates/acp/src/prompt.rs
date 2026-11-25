use std::sync::Arc;

use agent_client_protocol_schema as acp;
use claurst_api::streaming::{AnthropicStreamEvent, ContentDelta};
use claurst_core::types::Message;
use claurst_query::{ChildEventSink, QueryEvent, QueryOutcome};
use claurst_tools::ToolContext;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::connection::Connection;
use crate::permission::AcpPermissionHandler;
use crate::runtime::AgentRuntime;
use crate::sessions::SessionState;

pub async fn handle(
    runtime: Arc<AgentRuntime>,
    connection: Arc<Connection>,
    session: Arc<SessionState>,
    params: acp::PromptRequest,
) -> Result<acp::PromptResponse, acp::Error> {
    let user_text = render_prompt_blocks(&params.prompt);
    if user_text.trim().is_empty() {
        return Err(acp::Error::invalid_params());
    }

    let mut messages: Vec<Message> = {
        let guard = session.messages.lock();
        guard.clone()
    };
    messages.push(Message::user(user_text));

    let cancel = session.cancel_token.clone();

    let permission_handler: Arc<dyn claurst_core::PermissionHandler> =
        Arc::new(AcpPermissionHandler);
    let tool_ctx = ToolContext {
        working_dir: session.cwd.clone(),
        permission_mode: runtime.config.permission_mode.clone(),
        permission_handler,
        cost_tracker: runtime.cost_tracker.clone(),
        session_id: session.session_id.0.to_string(),
        file_history: session.file_history.clone(),
        current_turn: session.current_turn.clone(),
        non_interactive: false,
        mcp_manager: runtime.mcp_manager.clone(),
        config: runtime.config.clone(),
        managed_agent_config: runtime.config.managed_agents.clone(),
        completion_notifier: None,
        pending_permissions: Some(session.pending_permissions.clone()),
        permission_manager: Some(runtime.permission_manager.clone()),
        user_question_tx: None,
        cancel_token: cancel.clone(),
    };

    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<QueryEvent>();
    let forwarder = tokio::spawn(forward_events(connection.clone(), session.clone(), ev_rx));

    let outcome = claurst_query::CHILD_EVENTS
        .scope(
            child_sink(connection.clone(), session.clone()),
            claurst_query::run_query_loop(
                runtime.api_client.as_ref(),
                &mut messages,
                runtime.tools.as_slice(),
                &tool_ctx,
                &runtime.query_config,
                runtime.cost_tracker.clone(),
                Some(ev_tx),
                cancel,
                None,
            ),
        )
        .await;

    let _ = forwarder.await;

    {
        let mut guard = session.messages.lock();
        *guard = messages;
    }

    let stop_reason = match outcome {
        QueryOutcome::EndTurn { .. } => acp::StopReason::EndTurn,
        QueryOutcome::MaxTokens { .. } => acp::StopReason::MaxTokens,
        QueryOutcome::Cancelled => acp::StopReason::Cancelled,
        QueryOutcome::BudgetExceeded { .. } => acp::StopReason::MaxTurnRequests,
        QueryOutcome::Error(e) => {
            error!(error = ?e, "ACP: query loop errored");
            acp::StopReason::Refusal
        }
    };

    Ok(acp::PromptResponse::new(stop_reason))
}

fn render_prompt_blocks(blocks: &[acp::ContentBlock]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            acp::ContentBlock::Text(t) => parts.push(t.text.clone()),
            acp::ContentBlock::ResourceLink(link) => {
                parts.push(format!("[resource link: {}]", link.uri));
            }
            acp::ContentBlock::Resource(res) => {
                let json = serde_json::to_value(res).unwrap_or_default();
                if let Some(text) = json
                    .get("resource")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    parts.push(text.to_string());
                }
            }
            acp::ContentBlock::Image(_) | acp::ContentBlock::Audio(_) => {
                warn!("ACP: ignoring multimedia content block (capability not advertised)");
            }
            _ => {
                warn!("ACP: ignoring unknown content block variant");
            }
        }
    }
    parts.join("\n\n")
}

pub(crate) fn child_sink(
    connection: Arc<Connection>,
    session: Arc<SessionState>,
) -> ChildEventSink {
    Arc::new(move |parent, event| {
        let connection = connection.clone();
        let session = session.clone();
        Box::pin(async move {
            forward_tool_event(&connection, &session, event, Some(&parent)).await;
        })
    })
}

pub(crate) async fn forward_events(
    connection: Arc<Connection>,
    session: Arc<SessionState>,
    mut rx: mpsc::UnboundedReceiver<QueryEvent>,
) {
    let session_id = session.session_id.clone();
    while let Some(event) = rx.recv().await {
        match event {
            QueryEvent::Stream(AnthropicStreamEvent::ContentBlockDelta { delta, .. }) => {
                match delta {
                    ContentDelta::TextDelta { text } => {
                        send_text_chunk(&connection, &session_id, &text, false).await;
                    }
                    ContentDelta::ThinkingDelta { thinking } => {
                        send_text_chunk(&connection, &session_id, &thinking, true).await;
                    }
                    _ => {}
                }
            }
            QueryEvent::Error(msg) => {
                send_text_chunk(
                    &connection,
                    &session_id,
                    &format!("\n[error: {}]", msg),
                    false,
                )
                .await;
            }
            event => forward_tool_event(&connection, &session, event, None).await,
        }
    }
}

async fn forward_tool_event(
    connection: &Arc<Connection>,
    session: &Arc<SessionState>,
    event: QueryEvent,
    parent: Option<&str>,
) {
    match event {
        QueryEvent::ToolStart {
            tool_name,
            tool_id,
            input_json,
        } => {
            let raw_input = serde_json::from_str::<serde_json::Value>(&input_json).ok();
            announce_tool_call(connection, session, &tool_name, &tool_id, raw_input, parent).await;
        }
        QueryEvent::ToolEnd {
            tool_name: _,
            tool_id,
            result,
            is_error,
        } => {
            crate::background::report_tool_result(connection, session, &tool_id, &result, is_error)
                .await;
            session.announced_tools.lock().await.remove(&tool_id);
        }
        _ => {}
    }
}

pub(crate) async fn announce_tool_call(
    connection: &Arc<Connection>,
    session: &SessionState,
    tool_name: &str,
    tool_id: &str,
    raw_input: Option<serde_json::Value>,
    parent: Option<&str>,
) {
    let mut announced = session.announced_tools.lock().await;
    if !announced.insert(tool_id.to_string()) {
        return;
    }
    let mut tool_call = acp::ToolCall::new(
        acp::ToolCallId::new(tool_id),
        tool_title(tool_name, raw_input.as_ref()),
    )
    .kind(classify_tool_kind(tool_name))
    .status(acp::ToolCallStatus::InProgress)
    .meta(parent.map(child_meta));
    if let Some(input) = raw_input {
        tool_call = tool_call.raw_input(Some(input));
    }
    send_session_update(
        connection,
        &session.session_id,
        acp::SessionUpdate::ToolCall(tool_call),
    )
    .await;
}

pub(crate) fn child_meta(parent: &str) -> acp::Meta {
    let mut meta = acp::Meta::new();
    meta.insert(
        "gentParentToolCallId".to_string(),
        serde_json::json!(parent),
    );
    meta
}

async fn send_text_chunk(
    connection: &Arc<Connection>,
    session_id: &acp::SessionId,
    text: &str,
    is_thought: bool,
) {
    let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)));
    let update = if is_thought {
        acp::SessionUpdate::AgentThoughtChunk(chunk)
    } else {
        acp::SessionUpdate::AgentMessageChunk(chunk)
    };
    send_session_update(connection, session_id, update).await;
}

pub(crate) async fn send_session_update(
    connection: &Arc<Connection>,
    session_id: &acp::SessionId,
    update: acp::SessionUpdate,
) {
    let notif = acp::SessionNotification::new(session_id.clone(), update);
    if let Err(e) = connection.send_notification("session/update", notif).await {
        warn!(?e, "ACP: failed to send session/update");
    } else {
        debug!("ACP: sent session/update");
    }
}

pub(crate) fn classify_tool_kind(tool_name: &str) -> acp::ToolKind {
    match tool_name {
        "Read" | "FileRead" => acp::ToolKind::Read,
        "Edit" | "FileEdit" | "Write" | "FileWrite" | "BatchEdit" | "ApplyPatch" => {
            acp::ToolKind::Edit
        }
        "Bash" | "Shell" | "Execute" => acp::ToolKind::Execute,
        "WebFetch" | "WebSearch" => acp::ToolKind::Fetch,
        "Glob" | "Grep" | "GlobTool" => acp::ToolKind::Search,
        "Delete" | "Rm" => acp::ToolKind::Delete,
        "Move" | "Rename" => acp::ToolKind::Move,
        "Think" | "Sequential" => acp::ToolKind::Think,
        _ => acp::ToolKind::Other,
    }
}

fn tool_title(tool_name: &str, raw_input: Option<&serde_json::Value>) -> String {
    if let Some(input) = raw_input {
        for key in &["file_path", "path", "filename", "url", "pattern", "command"] {
            if let Some(v) = input.get(*key).and_then(|x| x.as_str()) {
                return format!("{}: {}", tool_name, v);
            }
        }
    }
    tool_name.to_string()
}
