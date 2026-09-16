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
    let mut all = vec![
        json!({"name":"gent_automations_list","description":"List Gent automations for a workspace","inputSchema":{"type":"object","required":["workspaceId"],"properties":{"workspaceId":{"type":"string"}}}}),
        json!({"name":"gent_automation_run","description":"Run one manual Gent automation","inputSchema":{"type":"object","required":["automationId"],"properties":{"automationId":{"type":"string"}}}}),
        json!({"name":"gent_forge_list","description":"List Gent Forge connectors for a workspace","inputSchema":{"type":"object","required":["workspaceId"],"properties":{"workspaceId":{"type":"string"}}}}),
        json!({"name":"gent_goal_update","description":"Report that the active Gent goal is complete, or blocked until the user helps","inputSchema":{"type":"object","required":["goalId","status"],"properties":{"goalId":{"type":"string"},"status":{"type":"string","enum":["complete","blocked"]},"note":{"type":"string"}}}}),
    ];
    all.extend(crate::chat_mcp_tools::tools());
    all.into_iter()
        .filter(|tool| {
            domain.is_none()
                || domain == Some("automations")
                    && tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("gent_automation"))
                || domain == Some("forge") && tool["name"] == "gent_forge_list"
                || domain == Some("goal") && tool["name"] == "gent_goal_update"
                || domain == Some("chat")
                    && tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("gent_chat_"))
        })
        .collect()
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
        Ok(value) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&value).unwrap_or_default()}],"structuredContent":value}})
        }
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":error.to_string()}],"isError":true}})
        }
    }
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
