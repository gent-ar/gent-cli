use std::path::PathBuf;

use serde_json::{Value, json};

use crate::chat_links_cli::{self, CallerConversation};

const CREATE_DESCRIPTION: &str = "Start a new Gent chat that works alongside this one. \
The new chat is a real conversation the user can open from the sidebar, with its own transcript \
and its own agent; it is not a background job and not a sub-agent. Give it a short label the user \
will recognize, and a prompt describing the work it should begin immediately. Returns its \
conversationId: keep it, because gent_chat_send and gent_chat_wait take it.";

const SEND_DESCRIPTION: &str = "Say something to another Gent chat you created or were created by. \
This is the only way to talk to it. If that chat is idle the message starts its next turn; if it \
is already working the message is steered into the turn in flight. Returns how it was delivered.";

const WAIT_DESCRIPTION: &str = "Block until the named Gent chats finish what they are doing, or \
until timeoutSeconds elapses. This call does not return while they are still working, so do not \
call it before you have given them something to do. On timeout it returns honestly, reporting \
each chat's phase and timedOut true, rather than failing. Returns each chat's final phase and the \
last thing it said.";

const LIST_DESCRIPTION: &str = "List the Gent chats in this conversation's thread: the chat that \
created this one, if any, and every chat this one created, with each one's label, current phase \
and last activity time. Use it to recover conversationIds after a restart.";

pub(crate) fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "gent_chat_create",
            "description": CREATE_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["label"],
                "properties": {
                    "label": {"type": "string", "description": "Short name the user sees for the new chat"},
                    "prompt": {"type": "string", "description": "What the new chat should start working on"},
                    "workspacePath": {"type": "string", "description": "Workspace for the new chat [default: this conversation's workspace]"},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_send",
            "description": SEND_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["targetConversationId", "message"],
                "properties": {
                    "targetConversationId": {"type": "string"},
                    "message": {"type": "string"},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_wait",
            "description": WAIT_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "required": ["conversationIds"],
                "properties": {
                    "conversationIds": {"type": "array", "items": {"type": "string"}, "maxItems": 16},
                    "timeoutSeconds": {"type": "integer", "minimum": 1, "maximum": 900},
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
        json!({
            "name": "gent_chat_list",
            "description": LIST_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conversationId": {"type": "string", "description": "Only needed when gentd cannot resolve the calling conversation"}
                }
            }
        }),
    ]
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
