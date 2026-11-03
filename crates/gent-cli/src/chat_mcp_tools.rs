use std::path::PathBuf;

use serde_json::Value;

use crate::chat_links_cli::{self, CallerConversation};

#[cfg(test)]
pub(crate) fn tools() -> Vec<Value> {
    gent_types::internal_tools::tools(Some("chat"))
}

pub(crate) async fn call(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: Option<&str>,
    name: &str,
    args: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let caller = caller(conversation_id, args)?;
    match name {
        "gent_chat_create" => {
            chat_links_cli::create(
                data_dir,
                no_autostart,
                &caller,
                text(args, "label")?,
                optional_text(args, "prompt"),
                optional_text(args, "workspacePath"),
            )
            .await
        }
        "gent_chat_send" => {
            chat_links_cli::send(
                data_dir,
                no_autostart,
                &caller,
                text(args, "targetConversationId")?,
                text(args, "message")?,
            )
            .await
        }
        "gent_chat_wait" => {
            chat_links_cli::wait(
                data_dir,
                no_autostart,
                &caller,
                conversation_ids(args)?,
                args.get("timeoutSeconds")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(300),
            )
            .await
        }
        "gent_chat_list" => chat_links_cli::list(data_dir, no_autostart, &caller).await,
        _ => Err("unknown Gent chat tool".into()),
    }
}

fn caller(
    conversation_id: Option<&str>,
    args: &Value,
) -> Result<CallerConversation, Box<dyn std::error::Error>> {
    conversation_id
        .map(str::to_owned)
        .or_else(|| optional_text(args, "conversationId"))
        .filter(|value| !value.trim().is_empty())
        .map(CallerConversation)
        .ok_or_else(|| {
            "gentd did not bind this MCP server to a conversation; pass conversationId".into()
        })
}

fn conversation_ids(args: &Value) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let values = args
        .get("conversationIds")
        .and_then(Value::as_array)
        .ok_or("conversationIds must be an array of conversation identifiers")?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "conversationIds must contain only strings".into())
        })
        .collect()
}

fn text(args: &Value, key: &str) -> Result<String, Box<dyn std::error::Error>> {
    optional_text(args, key).ok_or_else(|| format!("{key} is required").into())
}

fn optional_text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "chat_mcp_tools_tests.rs"]
mod tests;
