use gent_ports::PublicProviderRunError;
use sha2::{Digest, Sha256};

use super::CodexPromptRunner;

impl<L, P> CodexPromptRunner<L, P> {
    pub(super) fn selected_mcp_servers(
        &self,
        source_names: &[String],
    ) -> Result<Option<(serde_json::Value, String)>, PublicProviderRunError> {
        let Some((servers, _)) = self.current_mcp_servers()? else {
            return Ok(None);
        };
        if source_names.is_empty() {
            return Ok(Some((servers, String::new())));
        }
        let object = servers
            .as_object()
            .ok_or_else(|| PublicProviderRunError::Failed("Codex MCP config is invalid".into()))?;
        let mut selected = serde_json::Map::new();
        for name in source_names {
            let server = object.get(name).cloned().ok_or_else(|| {
                PublicProviderRunError::Failed("selected MCP source is not configured".into())
            })?;
            selected.insert(name.clone(), server);
        }
        let value = serde_json::Value::Object(selected);
        let digest = hex::encode(Sha256::digest(value.to_string().as_bytes()));
        Ok(Some((value, digest)))
    }

    pub(super) fn current_mcp_servers(
        &self,
    ) -> Result<Option<(serde_json::Value, String)>, PublicProviderRunError> {
        let Some(path) = &self.mcp_config else {
            return Ok(self
                .mcp_servers
                .clone()
                .map(|servers| (servers, String::new())));
        };
        let bytes = std::fs::read(path).map_err(|_| {
            PublicProviderRunError::Failed("Codex MCP config is unavailable".into())
        })?;
        if bytes.len() > 1024 * 1024 {
            return Err(PublicProviderRunError::Failed(
                "Codex MCP config is invalid".into(),
            ));
        }
        let servers = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.get("mcpServers").cloned())
            .filter(serde_json::Value::is_object)
            .ok_or_else(|| PublicProviderRunError::Failed("Codex MCP config is invalid".into()))?;
        Ok(Some((servers, hex::encode(Sha256::digest(bytes)))))
    }
}
