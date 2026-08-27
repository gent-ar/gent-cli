use std::path::{Path, PathBuf};

use gent_types::ToolSourceRecord;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_BYTES: u64 = 1024 * 1024;
const INTERNAL_SERVER_NAMES: [&str; 2] = ["gent-automations", "gent-forge"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneMcpConfig {
    path: PathBuf,
}

impl StandaloneMcpConfig {
    pub(crate) fn internal_only(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("standalone-mcp.json");
        std::fs::create_dir_all(data_dir).map_err(|_| "MCP config directory is unavailable")?;
        std::fs::write(&path, br#"{"mcpServers":{}}"#).map_err(|_| "MCP config is unavailable")?;
        Self::load(&path)?.with_internal_servers(data_dir)
    }

    pub(crate) fn with_internal_servers(self, data_dir: &Path) -> Result<Self, String> {
        let value = self.value()?;
        let mut servers = value
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("MCP config is invalid")?;
        let executable = gent_cli_executable()?;
        for (name, domain) in [("gent-automations", "automations"), ("gent-forge", "forge")] {
            servers.entry(name).or_insert_with(|| {
                serde_json::json!({
                    "command": executable.clone(),
                    "args": ["--data-dir", data_dir, "mcp", domain],
                    "env": {}
                })
            });
        }
        let path = data_dir.join("standalone-mcp.json");
        std::fs::create_dir_all(data_dir).map_err(|_| "MCP config directory is unavailable")?;
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"mcpServers": servers}))
                .map_err(|_| "MCP config is invalid")?,
        )
        .map_err(|_| "MCP config is unavailable")?;
        Self::load(&path)
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
                let valid = !id.is_empty()
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
        Ok(Self { path: path.into() })
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

fn gent_cli_executable() -> Result<String, String> {
    let current = std::env::current_exe().map_err(|_| "Gent executable is unavailable")?;
    let directory = current.parent().ok_or("Gent executable is unavailable")?;
    let name = if cfg!(windows) { "gent.exe" } else { "gent" };
    for directory in [Some(directory), directory.parent()].into_iter().flatten() {
        let sibling = directory.join(name);
        if sibling.is_file() {
            return Ok(sibling.to_string_lossy().into_owned());
        }
    }
    Err("Gent CLI executable is unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::StandaloneMcpConfig;

    #[test]
    fn loads_only_bounded_stdio_servers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"example":{"command":"node","args":["server.js"],"env":{"TOKEN":"secret"}}}}"#).unwrap();
        let config = StandaloneMcpConfig::load(&path).unwrap();
        assert_eq!(
            config.codex_servers().unwrap()["example"]["command"],
            "node"
        );
        assert_eq!(config.claurst_servers().unwrap()[0]["name"], "example");
        assert_eq!(
            config.claurst_servers().unwrap()[0]["env"],
            serde_json::json!([{"name":"TOKEN","value":"secret"}])
        );
        assert_eq!(
            config.claurst_settings_servers().unwrap()[0]["env"],
            serde_json::json!({"TOKEN":"secret"})
        );
        assert_eq!(
            config.claurst_servers().unwrap()[0]["args"],
            serde_json::json!(["server.js"])
        );
        let before = config.digest().unwrap();
        std::fs::write(
            &path,
            r#"{"mcpServers":{"example":{"command":"node","args":["updated.js"]}}}"#,
        )
        .unwrap();
        assert_ne!(before, config.digest().unwrap());
        assert_eq!(
            config.claurst_servers().unwrap()[0]["args"],
            serde_json::json!(["updated.js"])
        );
        assert_eq!(
            config.claurst_servers().unwrap()[0]["env"],
            serde_json::json!([])
        );
    }
}
