use std::path::{Path, PathBuf};

use gent_types::ToolSourceRecord;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "internal_mcp_descriptors.rs"]
mod descriptors;
pub(crate) use descriptors::InjectedInternalMcp;

const MAX_BYTES: u64 = 1024 * 1024;
pub(crate) use gent_types::internal_tools::INTERNAL_SERVER_NAMES;
pub(crate) const CHAT_SERVER_NAME: &str = "gent-chat";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneMcpConfig {
    path: PathBuf,
    injected: Option<InjectedInternalMcp>,
}

impl StandaloneMcpConfig {
    pub(crate) fn internal_only(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("standalone-mcp.json");
        std::fs::create_dir_all(data_dir).map_err(|_| "MCP config directory is unavailable")?;
        std::fs::write(&path, br#"{"mcpServers":{}}"#).map_err(|_| "MCP config is unavailable")?;
        Self::load(&path)?.with_internal_servers(data_dir)
    }

    pub(crate) fn with_internal_servers(self, data_dir: &Path) -> Result<Self, String> {
        let injected = descriptors::capture(data_dir)?;
        self.with_captured_internal_servers(data_dir, injected)
    }

    #[cfg(test)]
    pub(crate) fn with_fixture_internal_servers(
        self,
        data_dir: &Path,
        cli: &str,
    ) -> Result<Self, String> {
        self.with_captured_internal_servers(data_dir, InjectedInternalMcp::fixture(data_dir, cli))
    }

    fn with_captured_internal_servers(
        self,
        data_dir: &Path,
        injected: InjectedInternalMcp,
    ) -> Result<Self, String> {
        let value = self.value()?;
        let mut servers = value
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("MCP config is invalid")?;
        for (name, config) in injected
            .servers()
            .as_object()
            .ok_or("MCP config is invalid")?
        {
            servers.insert(name.clone(), config.clone());
        }
        let path = data_dir.join("standalone-mcp.json");
        std::fs::create_dir_all(data_dir).map_err(|_| "MCP config directory is unavailable")?;
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"mcpServers": servers}))
                .map_err(|_| "MCP config is invalid")?,
        )
        .map_err(|_| "MCP config is unavailable")?;
        let mut config = Self::load(&path)?;
        config.injected = Some(injected);
        Ok(config)
    }

    pub(crate) fn injection(&self) -> Option<InjectedInternalMcp> {
        self.injected.clone()
    }

    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::metadata(path).map_err(|_| "MCP config is unavailable")?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err("MCP config is invalid".into());
        }
        let value: Value =
            serde_json::from_slice(&std::fs::read(path).map_err(|_| "MCP config is unavailable")?)
                .map_err(|_| "MCP config is invalid")?;
        value
            .get("mcpServers")
            .and_then(Value::as_object)
            .ok_or("MCP config is invalid")?
            .iter()
            .try_for_each(|(id, config)| {
                let valid = !gent_types::internal_tools::sdk_alias(id)
                    && !id.is_empty()
                    && id.len() <= 256
                    && config
                        .get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|command| !command.is_empty() && command.len() <= 4096)
                    && config.get("args").is_none_or(|args| {
                        args.as_array().is_some_and(|values| {
                            values.len() <= 128
                                && values.iter().all(|value| {
                                    value.as_str().is_some_and(|value| value.len() <= 4096)
                                })
                        })
                    })
                    && config.get("env").is_none_or(|env| {
                        env.as_object().is_some_and(|values| {
                            values.len() <= 128
                                && values.iter().all(|(key, value)| {
                                    !key.is_empty()
                                        && key.len() <= 256
                                        && value.as_str().is_some_and(|value| value.len() <= 8192)
                                })
                        })
                    });
                valid.then_some(()).ok_or("MCP config is invalid")
            })?;
        Ok(Self {
            path: path.into(),
            injected: None,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn server_count(&self) -> Result<u16, String> {
        self.codex_servers()?
            .as_object()
            .map(|servers| servers.len().try_into().unwrap_or(u16::MAX))
            .ok_or_else(|| "MCP config is invalid".into())
    }

    pub(crate) fn server_names(&self) -> Result<Vec<String>, String> {
        let mut names = self
            .codex_servers()?
            .as_object()
            .ok_or("MCP config is invalid")?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    pub(crate) fn codex_servers(&self) -> Result<Value, String> {
        self.value().map(|value| value["mcpServers"].clone())
    }

    pub(crate) fn digest(&self) -> Result<String, String> {
        let bytes = self.bytes()?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub(crate) fn claurst_servers(&self) -> Result<Vec<Value>, String> {
        let servers = self
            .codex_servers()?
            .as_object()
            .cloned()
            .ok_or("MCP config is invalid")?;
        Ok(servers
            .iter()
            .map(|(name, config)| {
                let mut value = serde_json::Map::new();
                value.insert("name".into(), Value::String(name.clone()));
                if let Some(command) = config.get("command") {
                    value.insert("command".into(), command.clone());
                }
                value.insert(
                    "args".into(),
                    config
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| Value::Array(Vec::new())),
                );
                value.insert(
                    "env".into(),
                    Value::Array(
                        config
                            .get("env")
                            .and_then(Value::as_object)
                            .into_iter()
                            .flat_map(|env| {
                                env.iter().map(|(key, value)| {
                                    serde_json::json!({
                                        "name": key,
                                        "value": value,
                                    })
                                })
                            })
                            .collect(),
                    ),
                );
                Value::Object(value)
            })
            .collect())
    }

    pub(crate) fn claurst_settings_servers(&self) -> Result<Vec<Value>, String> {
        let servers = self
            .codex_servers()?
            .as_object()
            .cloned()
            .ok_or("MCP config is invalid")?;
        Ok(servers
            .iter()
            .map(|(name, config)| {
                let mut value = config.clone();
                value["name"] = Value::String(name.clone());
                value
            })
            .collect())
    }

    pub(crate) fn selected_claurst_servers(
        &self,
        sources: &[ToolSourceRecord],
    ) -> Result<Vec<Value>, String> {
        let servers = self
            .claurst_servers()?
            .into_iter()
            .map(|value| {
                (
                    value
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    value,
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        selected_server_names(sources)
            .into_iter()
            .map(|name| {
                servers
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| "selected MCP source is not configured".to_owned())
            })
            .collect()
    }

    pub(crate) fn selected_claurst_settings_servers(
        &self,
        sources: &[ToolSourceRecord],
    ) -> Result<Vec<Value>, String> {
        let servers = self
            .claurst_settings_servers()?
            .into_iter()
            .map(|value| {
                (
                    value
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    value,
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        selected_server_names(sources)
            .into_iter()
            .map(|name| {
                servers
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| "selected MCP source is not configured".to_owned())
            })
            .collect()
    }

    fn value(&self) -> Result<Value, String> {
        let bytes = self.bytes()?;
        serde_json::from_slice(&bytes).map_err(|_| "MCP config is invalid".into())
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        let _ = Self::load(&self.path)?;
        std::fs::read(&self.path).map_err(|_| "MCP config is unavailable".into())
    }
}

fn selected_server_names(sources: &[ToolSourceRecord]) -> Vec<String> {
    let mut names = sources
        .iter()
        .map(|source| source.source_name.clone())
        .collect::<Vec<_>>();
    names.extend(INTERNAL_SERVER_NAMES.map(str::to_owned));
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
#[path = "standalone_mcp_config_tests.rs"]
mod tests;
