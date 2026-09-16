use std::path::{Path, PathBuf};

use gent_ports::PublicProviderRunError;

const MAX_BYTES: u64 = 1024 * 1024;

pub(super) fn conversation_config(
    configured: Option<&Path>,
    names: &[String],
    conversation_id: &str,
    run_id: &str,
) -> Result<Option<PathBuf>, PublicProviderRunError> {
    let Some(path) = configured else {
        return Ok(None);
    };
    let data_dir = path
        .parent()
        .ok_or_else(|| failed("Claude MCP config is invalid"))?;
    let servers = configured_servers(path)?;
    let scoped = crate::conversation_scoped_mcp::scoped_servers(&servers, names, conversation_id)
        .map_err(PublicProviderRunError::Failed)?;
    crate::conversation_scoped_mcp::write_conversation_config(data_dir, run_id, &scoped)
        .map(Some)
        .map_err(PublicProviderRunError::Failed)
}

fn configured_servers(path: &Path) -> Result<serde_json::Value, PublicProviderRunError> {
    let metadata =
        std::fs::metadata(path).map_err(|_| failed("Claude MCP config is unavailable"))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(failed("Claude MCP config is invalid"));
    }
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path).map_err(|_| failed("Claude MCP config is unavailable"))?,
    )
    .map_err(|_| failed("Claude MCP config is invalid"))?;
    value
        .get("mcpServers")
        .filter(|servers| servers.is_object())
        .cloned()
        .ok_or_else(|| failed("Claude MCP config is invalid"))
}

fn failed(message: &str) -> PublicProviderRunError {
    PublicProviderRunError::Failed(message.into())
}

#[cfg(test)]
mod tests {
    use super::conversation_config;

    fn data_dir() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("standalone-mcp.json"),
            br#"{"mcpServers":{"gent-chat":{"command":"gent","args":["mcp","chat"]},"gent-goal":{"command":"gent","args":["mcp","goal"]}}}"#,
        )
        .unwrap();
        directory
    }

    fn servers(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
            ["mcpServers"]
            .clone()
    }

    #[test]
    fn a_claude_launch_writes_its_own_conversation_config_under_the_data_dir() {
        let directory = data_dir();
        let configured = directory.path().join("standalone-mcp.json");
        let written = conversation_config(Some(&configured), &[], "conversation-1", "run-1")
            .unwrap()
            .unwrap();

        assert_eq!(
            written.parent().unwrap().parent().unwrap(),
            directory.path()
        );
        assert_ne!(written, configured);
        let scoped = servers(&written);
        assert_eq!(
            scoped["gent-chat"]["args"],
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-1"])
        );
        assert_eq!(
            scoped["gent-goal"]["args"],
            serde_json::json!(["mcp", "goal"])
        );
        assert_eq!(
            servers(&configured)["gent-chat"]["args"],
            serde_json::json!(["mcp", "chat"])
        );
    }

    #[test]
    fn one_conversations_config_never_leaks_into_another() {
        let directory = data_dir();
        let configured = directory.path().join("standalone-mcp.json");
        let first = conversation_config(Some(&configured), &[], "conversation-1", "run-1")
            .unwrap()
            .unwrap();
        let second = conversation_config(Some(&configured), &[], "conversation-2", "run-2")
            .unwrap()
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(
            servers(&first)["gent-chat"]["args"],
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-1"])
        );
        assert_eq!(
            servers(&second)["gent-chat"]["args"],
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-2"])
        );
    }

    #[test]
    fn a_selected_source_keeps_the_conversation_and_refuses_an_unknown_server() {
        let directory = data_dir();
        let configured = directory.path().join("standalone-mcp.json");
        let written = conversation_config(
            Some(&configured),
            &["gent-chat".into()],
            "conversation-1",
            "run-1",
        )
        .unwrap()
        .unwrap();
        let scoped = servers(&written);
        assert!(scoped.get("gent-goal").is_none());
        assert_eq!(
            scoped["gent-chat"]["args"],
            serde_json::json!(["mcp", "chat", "--conversation-id", "conversation-1"])
        );
        assert!(
            conversation_config(
                Some(&configured),
                &["missing".into()],
                "conversation-1",
                "run-2",
            )
            .is_err()
        );
    }

    #[test]
    fn a_launch_without_a_conversation_is_refused_rather_than_unscoped() {
        let directory = data_dir();
        let configured = directory.path().join("standalone-mcp.json");
        assert!(conversation_config(Some(&configured), &[], "  ", "run-1").is_err());
        assert!(
            conversation_config(None, &[], "conversation-1", "run-1")
                .unwrap()
                .is_none()
        );
    }
}
