use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::standalone_mcp_config::CHAT_SERVER_NAME;

const CONVERSATION_ARGUMENT: &str = "--conversation-id";
const CONVERSATION_CONFIG_DIRECTORY: &str = "conversation-mcp";

pub(crate) fn scoped_servers(
    servers: &Value,
    names: &[String],
    conversation_id: &str,
) -> Result<Value, String> {
    if conversation_id.trim().is_empty() {
        return Err("a provider launch needs its conversation identity".into());
    }
    let entries = servers.as_object().ok_or("MCP config is invalid")?;
    let selected = if names.is_empty() {
        servers.clone()
    } else {
        let mut picked = Map::new();
        for name in names {
            picked.insert(
                name.clone(),
                entries
                    .get(name)
                    .cloned()
                    .ok_or("selected MCP source is not configured")?,
            );
        }
        Value::Object(picked)
    };
    Ok(scope_server_map(&selected, conversation_id))
}

pub(crate) fn write_conversation_config(
    data_dir: &Path,
    run_id: &str,
    servers: &Value,
) -> Result<PathBuf, String> {
    let name = run_id.trim();
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("a conversation MCP config needs a bounded run identity".into());
    }
    let directory = data_dir.join(CONVERSATION_CONFIG_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .map_err(|_| "conversation MCP config directory is unavailable".to_owned())?;
    let path = directory.join(format!("{name}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"mcpServers": servers}))
            .map_err(|_| "conversation MCP config is invalid".to_owned())?,
    )
    .map_err(|_| "conversation MCP config is unavailable".to_owned())?;
    Ok(path)
}

pub(crate) fn scope_server_map(servers: &Value, conversation_id: &str) -> Value {
    let Some(entries) = servers.as_object() else {
        return servers.clone();
    };
    let mut scoped = Map::new();
    for (name, config) in entries {
        scoped.insert(
            name.clone(),
            if name == CHAT_SERVER_NAME {
                with_conversation(config, conversation_id)
            } else {
                config.clone()
            },
        );
    }
    Value::Object(scoped)
}

pub(crate) fn scope_named_entries(entries: Vec<Value>, conversation_id: &str) -> Vec<Value> {
    entries
        .into_iter()
        .map(|entry| {
            if entry.get("name").and_then(Value::as_str) == Some(CHAT_SERVER_NAME) {
                with_conversation(&entry, conversation_id)
            } else {
                entry
            }
        })
        .collect()
}

fn with_conversation(config: &Value, conversation_id: &str) -> Value {
    let mut scoped = config.clone();
    let mut arguments = scoped
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if arguments
        .iter()
        .any(|value| value.as_str() == Some(CONVERSATION_ARGUMENT))
    {
        return scoped;
    }
    arguments.push(Value::String(CONVERSATION_ARGUMENT.into()));
    arguments.push(Value::String(conversation_id.to_owned()));
    scoped["args"] = Value::Array(arguments);
    scoped
}

#[cfg(test)]
mod tests {
    use super::{scope_named_entries, scope_server_map};
    use serde_json::json;

    #[test]
    fn only_the_chat_server_learns_the_conversation() {
        let servers = json!({
            "gent-chat": {"command": "gent", "args": ["--data-dir", "/data", "mcp", "chat"]},
            "gent-goal": {"command": "gent", "args": ["--data-dir", "/data", "mcp", "goal"]}
        });
        let scoped = scope_server_map(&servers, "conversation-1");
        assert_eq!(
            scoped["gent-chat"]["args"],
            json!([
                "--data-dir",
                "/data",
                "mcp",
                "chat",
                "--conversation-id",
                "conversation-1"
            ])
        );
        assert_eq!(scoped["gent-goal"], servers["gent-goal"]);
    }

    #[test]
    fn scoping_twice_never_repeats_the_argument() {
        let servers = json!({"gent-chat": {"command": "gent", "args": ["mcp", "chat"]}});
        let once = scope_server_map(&servers, "conversation-1");
        let twice = scope_server_map(&once, "conversation-2");
        assert_eq!(
            twice["gent-chat"]["args"],
            json!(["mcp", "chat", "--conversation-id", "conversation-1"])
        );
    }

    #[test]
    fn named_entries_carry_the_conversation_for_claurst() {
        let entries = vec![
            json!({"name": "gent-chat", "command": "gent", "args": ["mcp", "chat"]}),
            json!({"name": "gent-forge", "command": "gent", "args": ["mcp", "forge"]}),
        ];
        let scoped = scope_named_entries(entries, "conversation-9");
        assert_eq!(
            scoped[0]["args"],
            json!(["mcp", "chat", "--conversation-id", "conversation-9"])
        );
        assert_eq!(scoped[1]["args"], json!(["mcp", "forge"]));
    }

    #[test]
    fn a_chat_server_without_arguments_still_gains_the_conversation() {
        let servers = json!({"gent-chat": {"command": "gent"}});
        assert_eq!(
            scope_server_map(&servers, "conversation-1")["gent-chat"]["args"],
            json!(["--conversation-id", "conversation-1"])
        );
    }
}
