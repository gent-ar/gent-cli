//! Fixed configured-roster capture for the patched Claurst ACP client.
//! Original stdio commands/arguments/env remain only in host snapshots.
use super::{
    BTreeMap, Deserialize, Digest, Document, HostMcpConfig, HostStdioServer, MAX_BYTES, Path, Read,
    Sha256, WorkerMcpConfig, Write, Zeroize, Zeroizing, open_locked, unique_map,
};
#[derive(Deserialize)]
struct Settings {
    config: Config,
}
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    mcp_servers: Vec<Server>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default, deserialize_with = "unique_map")]
    env: BTreeMap<String, String>,
    #[serde(rename = "type", default)]
    transport: Option<String>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.command.zeroize();
        for value in &mut self.args {
            value.zeroize();
        }
        for value in self.env.values_mut() {
            value.zeroize();
        }
    }
}
impl HostMcpConfig {
    pub(crate) fn capture_claurst(path: &Path) -> Result<Self, String> {
        let mut source = open_locked(path)?;
        let mut bytes = Zeroizing::new(Vec::new());
        if source
            .metadata()
            .map_err(|_| "Claurst MCP settings unavailable")?
            .len()
            > MAX_BYTES
        {
            return Err("Claurst MCP settings exceed bound".into());
        }
        (&mut source)
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Claurst MCP settings unavailable")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Claurst MCP settings exceed bound".into());
        }
        let mut settings: Settings =
            serde_json::from_slice(&bytes).map_err(|_| "Claurst MCP settings violate contract")?;
        if settings.config.mcp_servers.len() > 32 {
            return Err("Claurst MCP server count exceeds bound".into());
        }
        let mut servers = BTreeMap::new();
        for server in &mut settings.config.mcp_servers {
            if servers.contains_key(&server.name) {
                return Err("Claurst MCP server duplicated".into());
            }
            servers.insert(
                std::mem::take(&mut server.name),
                HostStdioServer {
                    command: std::mem::take(&mut server.command),
                    args: std::mem::take(&mut server.args),
                    env: std::mem::take(&mut server.env),
                    transport: server.transport.take(),
                },
            );
        }
        Self::from_document(
            Document { servers },
            source,
            hex::encode(Sha256::digest(&bytes)),
        )
    }
}
pub(super) fn worker(clients: serde_json::Map<String, serde_json::Value>) -> serde_json::Value {
    let entries:Vec<_>=clients.into_iter().map(|(name,value)|serde_json::json!({"name":name,"type":"http","command":null,"args":[],"url":value["url"].as_str().expect("internally constructed URL").replace("${ANTHROPIC_BASE_URL}","${GENT_MCP_BRIDGE_ORIGIN}"),"env":{"MCP_AUTH_TOKEN":"${GENT_MCP_BRIDGE_TOKEN}"}})).collect();
    serde_json::json!({"mcpServers":entries})
}
pub(super) fn publish(private_root: &Path, bytes: &[u8]) -> Result<WorkerMcpConfig, String> {
    let path = private_root.join(format!("mcp-client-{}.json", uuid::Uuid::new_v4().simple()));
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "Contained MCP client configuration cannot be created.")?;
    output
        .write_all(bytes)
        .and_then(|()| output.sync_all())
        .map_err(|_| "Contained MCP client configuration cannot be published.")?;
    drop(output);
    let handle = open_locked(&path)?;
    Ok(WorkerMcpConfig { path, handle })
}
#[cfg(test)]
#[path = "contained_claurst_mcp_tests.rs"]
mod tests;
