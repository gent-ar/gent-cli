// Tool execution helpers: argument parsing, permission gating, and the
// single-tool / batch execution paths. Extracted from lib.rs (issue #232).
// Behavior-preserving move.

use crate::*;

/// Parse the accumulated JSON arguments of a streamed tool call.
///
/// Providers stream a tool call's arguments as a sequence of partial-JSON
/// deltas which we concatenate into a single buffer. A well-behaved
/// no-argument call yields an empty (or whitespace-only) buffer, which we
/// map to an empty object. Any *non-empty* buffer that fails to parse is
/// returned as an error rather than being silently replaced with `{}` — a
/// truncated stream must never cause a tool (e.g. Edit/Write) to run with
/// empty arguments (issue #215).
pub(crate) fn parse_tool_args(json_str: &str) -> Result<Value, serde_json::Error> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(trimmed)
}

/// Whether a `PermissionLevel` must be gated by the central backstop.
///
/// Only `None` and `ReadOnly` are exempt; every other level (`Write`,
/// `Execute`, `Dangerous`, `Forbidden`) represents a side-effecting action that
/// the backstop must confirm before it runs.
pub(crate) fn permission_level_is_gated(level: PermissionLevel) -> bool {
    !matches!(level, PermissionLevel::None | PermissionLevel::ReadOnly)
}

/// Synthesize a human-readable permission description for a tool that does not
/// gate itself, surfacing the tool name and a truncated preview of its input so
/// the user can see what is about to run.
pub(crate) fn synthesize_permission_description(name: &str, input: &Value) -> String {
    let rendered = serde_json::to_string(input).unwrap_or_default();
    let preview: String = rendered.chars().take(200).collect();
    if preview.is_empty() || preview == "{}" || preview == "null" {
        format!("Run tool '{}'", name)
    } else {
        format!("Run tool '{}' with input: {}", name, preview)
    }
}

/// Execute a single tool invocation.
pub(crate) async fn execute_tool(
    id: &str,
    name: &str,
    input: &Value,
    tools: &[Box<dyn Tool>],
    ctx: &ToolContext,
) -> ToolResult {
    let tool = tools.iter().find(|t| t.name() == name);

    match tool {
        Some(tool) => claurst_tools::TOOL_CALL.scope(claurst_tools::ToolCallBinding::new(id, name, input), async {
            debug!(tool = name, "Executing tool");
            // Central permission backstop (issue #210): if a tool does not gate
            // itself (`self_gates() == false`) and requires a gated permission
            // level, prompt here BEFORE executing. On denial, return a blocked
            // result WITHOUT running the tool. Tools that already prompt
            // internally opt out via `self_gates() == true` (no double-prompt),
            // and read-only / no-permission tools are skipped. This makes a tool
            // that forgets to gate itself secure by default.
            if !tool.self_gates() && permission_level_is_gated(tool.permission_level()) {
                let description = synthesize_permission_description(name, input);
                if let Err(e) = ctx.check_permission(name, &description, false) {
                    warn!(tool = name, "Tool blocked by central permission backstop");
                    return ToolResult::error(e.to_string());
                }
            }
            tool.execute(input.clone(), ctx).await
        }).await,
        None => {
            warn!(tool = name, "Unknown tool requested");
            ToolResult::error(format!("Unknown tool: {}", name))
        }
    }
}

/// Run a batch of tool-execution futures concurrently, abandoning them promptly
/// if `cancel_token` fires (issue #218).
///
/// Returns exactly one `ToolResult` per input future, in order, plus a bool that
/// is `true` iff the batch was cancelled before every tool finished. On the
/// happy path (no cancellation) this is `join_all` and the results are the real
/// tool outputs. On cancellation the in-flight futures are dropped (abandoned)
/// and every position is filled with a synthetic cancelled `ToolResult` so the
/// caller can still answer every `tool_use` and keep the message history valid.
pub(crate) async fn run_tool_batch<F>(
    exec_futures: Vec<F>,
    cancel_token: &tokio_util::sync::CancellationToken,
) -> (Vec<ToolResult>, bool)
where
    F: std::future::Future<Output = ToolResult>,
{
    let count = exec_futures.len();
    tokio::select! {
        results = futures::future::join_all(exec_futures) => (results, false),
        _ = cancel_token.cancelled() => {
            let cancelled = (0..count)
                .map(|_| ToolResult::error(TOOL_CANCELLED_MSG))
                .collect();
            (cancelled, true)
        }
    }
}

pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: Value,
    pub(crate) refused: Option<ToolResult>,
}

pub(crate) async fn run_tool_round(
    calls: Vec<ToolCall>,
    tools: &[Box<dyn Tool>],
    tool_ctx: &ToolContext,
    event_tx: &Option<mpsc::UnboundedSender<QueryEvent>>,
) -> (Message, bool) {
    let mut prepared = Vec::with_capacity(calls.len());
    for mut call in calls {
        if let Some(tx) = event_tx {
            let _ = tx.send(QueryEvent::ToolStart {
                tool_name: call.name.clone(),
                tool_id: call.id.clone(),
                input_json: call.input.to_string(),
            });
        }
        if call.refused.is_none() {
            let hook_ctx = claurst_core::hooks::HookContext {
                event: "PreToolUse".to_string(),
                tool_name: Some(call.name.clone()),
                tool_input: Some(call.input.clone()),
                tool_output: None,
                is_error: None,
                session_id: Some(tool_ctx.session_id.clone()),
            };
            let pre_outcome = claurst_core::hooks::run_hooks(
                &tool_ctx.config.hooks,
                claurst_core::config::HookEvent::PreToolUse,
                &hook_ctx,
                &tool_ctx.working_dir,
            )
            .await;
            let plugin_pre_outcome = claurst_plugins::run_global_pre_tool_hook(&call.name, &call.input);
            call.refused = if let claurst_core::hooks::HookOutcome::Blocked(reason) = pre_outcome {
                warn!(tool = %call.name, reason = %reason, "PreToolUse hook blocked execution");
                Some(ToolResult::error(format!("Blocked by hook: {}", reason)))
            } else if let claurst_plugins::HookOutcome::Deny(reason) = plugin_pre_outcome {
                warn!(tool = %call.name, reason = %reason, "Plugin PreToolUse hook blocked execution");
                Some(ToolResult::error(format!("Blocked by plugin hook: {}", reason)))
            } else {
                None
            };
        }
        prepared.push(call);
    }

    let exec_futures: Vec<_> = prepared
        .iter()
        .map(|call| match &call.refused {
            Some(refused) => {
                let refused = refused.clone();
                futures::future::Either::Left(async move { refused })
            }
            None => futures::future::Either::Right(execute_tool(&call.id, &call.name, &call.input, tools, tool_ctx)),
        })
        .collect();
    let (exec_results, cancelled) = run_tool_batch(exec_futures, &tool_ctx.cancel_token).await;
    let cancelled = cancelled || tool_ctx.cancel_token.is_cancelled();

    let mut result_blocks = Vec::with_capacity(prepared.len());
    for (call, result) in prepared.iter().zip(exec_results) {
        let result = if cancelled && result.is_error {
            ToolResult::error(TOOL_CANCELLED_MSG)
        } else {
            result
        };
        if !cancelled {
            let post_ctx = claurst_core::hooks::HookContext {
                event: "PostToolUse".to_string(),
                tool_name: Some(call.name.clone()),
                tool_input: Some(call.input.clone()),
                tool_output: Some(result.content.clone()),
                is_error: Some(result.is_error),
                session_id: Some(tool_ctx.session_id.clone()),
            };
            claurst_core::hooks::run_hooks(
                &tool_ctx.config.hooks,
                claurst_core::config::HookEvent::PostToolUse,
                &post_ctx,
                &tool_ctx.working_dir,
            )
            .await;
            claurst_plugins::run_global_post_tool_hook(&call.name, &call.input, &result.content, result.is_error);
        }
        if let Some(tx) = event_tx {
            let _ = tx.send(QueryEvent::ToolEnd {
                tool_name: call.name.clone(),
                tool_id: call.id.clone(),
                result: result.content.clone(),
                is_error: result.is_error,
            });
        }
        result_blocks.push(ContentBlock::ToolResult {
            tool_use_id: call.id.clone(),
            content: ToolResultContent::Text(result.content),
            is_error: if result.is_error { Some(true) } else { None },
        });
    }
    (Message::user_blocks(result_blocks), cancelled)
}

/// Load persisted todos for `session_id` and return a nudge string if any are
/// incomplete (status != "completed"). Returns empty string otherwise.
pub(crate) fn build_todo_nudge(session_id: &str) -> String {
    let todos = claurst_tools::todo_write::load_todos(session_id);
    let incomplete_count = todos
        .iter()
        .filter(|t| t["status"].as_str() != Some("completed"))
        .count();
    if incomplete_count == 0 {
        String::new()
    } else {
        format!(
            "You have {} incomplete task{} in your TodoWrite list. \
             Make sure to complete all tasks before ending your response.",
            incomplete_count,
            if incomplete_count == 1 { "" } else { "s" }
        )
    }
}
