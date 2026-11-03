//! Host-only MCP snapshots and credential-free contained client configuration.
//! Transport ownership, executable trust and per-call permission fencing belong
//! to the host bridge. This module never launches a server or relaxes a grant.
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::{Zeroize, Zeroizing};

#[path = "contained_claurst_mcp.rs"]
mod claurst;

const MAX_BYTES: u64 = 1024 * 1024;
pub(crate) const ROUTE_PREFIX: &str = "/__gent_mcp/";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostStdioServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "unique_map")]
    pub env: BTreeMap<String, String>,
    #[serde(rename = "type", default)]
    transport: Option<String>,
}
impl fmt::Debug for HostStdioServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HostStdioServer { redacted }")
    }
}
impl Drop for HostStdioServer {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(rename = "mcpServers", deserialize_with = "unique_map")]
    servers: BTreeMap<String, HostStdioServer>,
}

fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Unique<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a bounded unique MCP map")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if result.len() >= 32 || result.insert(key, value).is_some() {
                    return Err(de::Error::custom("duplicate or oversized MCP map"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

pub(crate) struct HostMcpConfig {
    servers: BTreeMap<String, HostStdioServer>,
    _source: File,
    digest: String,
}
impl fmt::Debug for HostMcpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostMcpConfig")
            .field("server_count", &self.servers.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct WorkerMcpConfig {
    pub path: PathBuf,
    pub handle: File,
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
fn text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.contains('\0')
}
fn physical(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Contained MCP paths must be absolute.".into());
    }
    for component in path.ancestors() {
        let metadata = std::fs::symlink_metadata(component)
            .map_err(|_| "Contained MCP path is unavailable.")?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Contained MCP redirected paths are unsupported.".into());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("Contained MCP redirected paths are unsupported.".into());
        }
    }
    Ok(())
}
fn open_locked(path: &Path) -> Result<File, String> {
    physical(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options
        .open(path)
        .map_err(|_| "Contained MCP configuration cannot be locked.")?;
    if !file
        .metadata()
        .map_err(|_| "Contained MCP configuration is unavailable.")?
        .is_file()
    {
        return Err("Contained MCP configuration must be a file.".into());
    }
    Ok(file)
}

impl HostMcpConfig {
    pub(crate) fn capture(path: &Path) -> Result<Self, String> {
        let mut source = open_locked(path)?;
        if source
            .metadata()
            .map_err(|_| "Contained MCP configuration is unavailable.")?
            .len()
            > MAX_BYTES
        {
            return Err("Contained MCP configuration exceeds its bound.".into());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        (&mut source)
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Contained MCP configuration cannot be read.")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Contained MCP configuration exceeds its bound.".into());
        }
        let document: Document = serde_json::from_slice(&bytes)
            .map_err(|_| "Contained MCP configuration violates its bounded stdio contract.")?;
        Self::from_document(document, source, hex::encode(Sha256::digest(&bytes)))
    }
    fn from_document(document: Document, source: File, digest: String) -> Result<Self, String> {
        let mut aliases = std::collections::BTreeSet::new();
        for (key, server) in &document.servers {
            if !aliases.insert(key.replace('-', "_")) {
                return Err("Contained MCP server aliases collide.".into());
            }
            if !name(key)
                || !text(&server.command, 32768)
                || !Path::new(&server.command).is_absolute()
                || server
                    .transport
                    .as_deref()
                    .is_some_and(|kind| kind != "stdio")
                || server.args.len() > 128
                || server.args.iter().any(|arg| !text(arg, 16384))
                || server.env.iter().any(|(key, value)| {
                    key.is_empty()
                        || key.len() > 128
                        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        || !text(value, 32768)
                })
            {
                return Err("Contained MCP server is outside the bounded stdio contract.".into());
            }
        }
        Ok(Self {
            servers: document.servers,
            _source: source,
            digest,
        })
    }
    pub(crate) fn servers(&self) -> &BTreeMap<String, HostStdioServer> {
        &self.servers
    }
    pub(crate) fn source_sha256(&self) -> &str {
        &self.digest
    }

    /// The caller must register every route with its fenced host bridge first.
    /// Names are preserved exactly; no unsupported server is silently omitted.
    pub(crate) fn materialize(
        &self,
        private_root: &Path,
        routes: &BTreeMap<String, String>,
    ) -> Result<WorkerMcpConfig, String> {
        self.materialize_format(private_root, routes, false)
    }
    pub(crate) fn materialize_claurst(
        &self,
        private_root: &Path,
        routes: &BTreeMap<String, String>,
    ) -> Result<WorkerMcpConfig, String> {
        self.materialize_format(private_root, routes, true)
    }
    fn materialize_format(
        &self,
        private_root: &Path,
        routes: &BTreeMap<String, String>,
        claurst: bool,
    ) -> Result<WorkerMcpConfig, String> {
        physical(private_root)?;
        if !private_root.is_dir()
            || routes.len() != self.servers.len()
            || !routes.keys().eq(self.servers.keys())
        {
            return Err("Contained MCP bridge routes must cover the exact server set.".into());
        }
        let mut clients = serde_json::Map::new();
        let mut seen = std::collections::BTreeSet::new();
        for (name, id) in routes {
            if id.len() != 32
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                || !seen.insert(id)
            {
                return Err(
                    "Contained MCP bridge route identifiers are invalid or duplicated.".into(),
                );
            }
            clients.insert(
                name.clone(),
                serde_json::json!({"type":"http",
                "url":format!("${{ANTHROPIC_BASE_URL}}{ROUTE_PREFIX}{id}"),
                "headers":{"Authorization":"Bearer ${GENT_MCP_BRIDGE_TOKEN}"}}),
            );
        }
        let document = if claurst {
            claurst::worker(clients)
        } else {
            serde_json::json!({"mcpServers":clients})
        };
        let bytes = serde_json::to_vec(&document)
            .map_err(|_| "Contained MCP client configuration cannot be encoded.")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Contained MCP client configuration exceeds its bound.".into());
        }
        claurst::publish(private_root, &bytes)
    }
}

#[cfg(test)]
#[path = "contained_claude_mcp_tests.rs"]
mod tests;
