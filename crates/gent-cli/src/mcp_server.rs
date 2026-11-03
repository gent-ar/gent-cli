use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};

use serde_json::{Value, json};

pub(crate) async fn run(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    domain: Option<String>,
    conversation_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let input = io::stdin();
    let mut output = io::BufWriter::new(io::stdout());
    for line in input.lock().lines() {
        let line = line?;
        let request: Value = serde_json::from_str(&line)?;
        if request.get("id").is_none() {
            let _ = dispatch(
                data_dir.clone(),
                no_autostart,
                domain.as_deref(),
                conversation_id.as_deref(),
                request,
            )
            .await;
            continue;
        }
        let response = dispatch(
            data_dir.clone(),
            no_autostart,
            domain.as_deref(),
            conversation_id.as_deref(),
            request,
        )
        .await;
        writeln!(output, "{}", serde_json::to_string(&response)?)?;
        output.flush()?;
    }
    Ok(())
}

async fn dispatch(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    domain: Option<&str>,
    conversation_id: Option<&str>,
    request: Value,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"gent","version":env!("CARGO_PKG_VERSION")}}})
        }
        Some("notifications/initialized") => Value::Null,
        Some("tools/list") => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools(domain)}}),
        Some("tools/call") => {
            call_tool(
                data_dir,
                no_autostart,
                domain,
                conversation_id,
                id,
                request.get("params").cloned().unwrap_or_default(),
            )
            .await
        }
        _ => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
    }
}

fn tools(domain: Option<&str>) -> Vec<Value> {
    gent_types::internal_tools::tools(domain)
}

fn tool_available(domain: Option<&str>, name: &str) -> bool {
    tools(domain)
        .iter()
        .any(|tool| tool["name"].as_str() == Some(name))
}

async fn call_tool(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    domain: Option<&str>,
    conversation_id: Option<&str>,
    id: Value,
    params: Value,
) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or_default();
    if !tool_available(domain, name) {
        return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"tool is not available from this Gent MCP server"}});
    }
    let result = match name {
        "gent_automations_list" => crate::automation_cli::list(
            data_dir,
            no_autostart,
            args.get("workspaceId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        )
        .await
        .map(|value| json!(value)),
        "gent_automation_run" => crate::automation_cli::run(
            data_dir,
            no_autostart,
            gent_types::AutomationId(
                args.get("automationId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            ),
        )
        .await
        .map(|value| json!(value)),
        "gent_forge_list" => crate::forge_cli::list(
            data_dir,
            no_autostart,
            args.get("workspaceId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        )
        .await
        .map(|value| json!(value)),
        "gent_goal_update" => goal_update(data_dir, no_autostart, &args).await,
        "gent_chat_create" | "gent_chat_send" | "gent_chat_wait" | "gent_chat_list" => {
            crate::chat_mcp_tools::call(data_dir, no_autostart, conversation_id, name, &args).await
        }
        _ => Err("unknown Gent tool".into()),
    };
    match result {
        Ok(value) => successful_tool_result(id, value),
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":error.to_string()}],"isError":true}})
        }
    }
}

// The negotiated 2025 MCP schemas require structuredContent to be an object.
// Preserve the exact JSON text result and every original object field.
fn successful_tool_result(id: Value, value: Value) -> Value {
    let text = serde_json::to_string(&value).unwrap_or_default();
    let structured = if value.is_object() {
        value
    } else {
        json!({"result": value})
    };
    json!({"jsonrpc":"2.0","id":id,"result":{
        "content":[{"type":"text","text":text}],"structuredContent":structured
    }})
}

async fn goal_update(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let outcome = match args.get("status").and_then(Value::as_str) {
        Some("complete") => gent_types::GoalReportOutcome::Complete,
        Some("blocked") => gent_types::GoalReportOutcome::Blocked,
        _ => return Err("status must be complete or blocked".into()),
    };
    let goal = crate::goal_cli::report(
        data_dir,
        no_autostart,
        args.get("goalId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        outcome,
        args.get("note").and_then(Value::as_str).map(str::to_owned),
    )
    .await?;
    Ok(json!(goal))
}

#[cfg(test)]
mod tests {
    use super::{tool_available, tools};

    #[test]
    fn internal_servers_expose_only_their_owned_tools() {
        assert_eq!(tools(Some("automations")).len(), 2);
        assert!(tool_available(Some("automations"), "gent_automations_list"));
        assert!(tool_available(Some("automations"), "gent_automation_run"));
        assert!(!tool_available(Some("automations"), "gent_forge_list"));
        assert_eq!(tools(Some("forge")).len(), 1);
        assert!(tool_available(Some("forge"), "gent_forge_list"));
        assert!(!tool_available(Some("forge"), "gent_automation_run"));
        assert_eq!(tools(Some("goal")).len(), 1);
        assert!(tool_available(Some("goal"), "gent_goal_update"));
        assert!(!tool_available(Some("goal"), "gent_forge_list"));
    }
}

#[cfg(test)]
mod structured_result_tests {
    use super::*;
    #[test]
    fn mcp_structured_result_preserves_objects_and_wraps_lists_and_scalars() {
        for value in [
            json!([]),
            json!([1, 2]),
            json!(null),
            json!("text"),
            json!(true),
            json!(42),
            json!({"items":[]}),
        ] {
            let reply = successful_tool_result(json!(7), value.clone());
            assert_eq!(reply["id"], 7);
            assert_eq!(
                reply["result"]["content"][0]["text"],
                serde_json::to_string(&value).unwrap()
            );
            assert!(reply["result"]["structuredContent"].is_object());
            if value.is_object() {
                assert_eq!(reply["result"]["structuredContent"], value);
            } else {
                assert_eq!(reply["result"]["structuredContent"]["result"], value);
            }
            assert!(reply["result"].get("isError").is_none());
        }
    }
}
