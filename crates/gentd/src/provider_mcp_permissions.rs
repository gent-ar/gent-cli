//! One-use bridge grants from actual host-to-SDK permission responses.
//! Worker control requests alone never grant authority. No raw arguments persist.
use super::mcp::Error;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[path = "provider_mcp_acp.rs"]
mod acp;

const MAX_FRAME: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 256;
#[derive(Default)]
pub struct PermissionRelay {
    turn: String,
    incoming: Vec<u8>,
    oversized: bool,
    requests: BTreeMap<String, (String, String)>,
    grants: Vec<(String, String)>,
    acp_session: Option<String>,
    acp_options: BTreeMap<String, Vec<String>>,
    claims: BTreeMap<String, (String, String)>,
}
fn digest(v: &Value) -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(v).map_err(|_| Error::Protocol)?)
    ))
}
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str().filter(|s| !s.is_empty() && s.len() <= 256)
}
fn qualified(name: &str) -> Result<String, Error> {
    let (server, tool) = name
        .strip_prefix("mcp__")
        .and_then(|v| v.split_once("__"))
        .ok_or(Error::Protocol)?;
    if server.is_empty() || tool.is_empty() {
        return Err(Error::Protocol);
    }
    Ok(format!("mcp__{}__{tool}", server.replace('-', "_")))
}
impl PermissionRelay {
    /// Supplied by the host's live active-turn authority, never the SDK.
    pub fn set_turn(&mut self, turn: &str) -> Result<(), Error> {
        if turn.is_empty() || turn.len() > 512 {
            return Err(Error::Protocol);
        }
        if self.turn != turn {
            self.incoming.clear();
            self.oversized = false;
            self.requests.clear();
            self.acp_options.clear();
            self.acp_session = None;
            self.grants.clear();
            self.claims.clear();
            self.turn = turn.into();
        }
        Ok(())
    }
    pub fn observe_sdk_stdout(&mut self, chunk: &[u8]) -> Result<(), Error> {
        for byte in chunk {
            if self.oversized {
                if *byte == b'\n' {
                    self.oversized = false;
                }
                continue;
            }
            // Oversized SDK output is still handled by its existing codec. It
            // cannot mint a bridge permission; do not accumulate it a second time.
            if self.incoming.len() >= MAX_FRAME {
                self.incoming.clear();
                self.oversized = *byte != b'\n';
                continue;
            }
            self.incoming.push(*byte);
            if *byte != b'\n' {
                continue;
            }
            let frame: Value =
                serde_json::from_slice(&self.incoming).map_err(|_| Error::Protocol)?;
            self.incoming.clear();
            if let Some(request) = acp::request(&frame, self.acp_session.as_deref())? {
                if self.requests.len() >= MAX_ENTRIES || self.requests.contains_key(&request.key) {
                    return Err(Error::Protocol);
                }
                self.requests.insert(
                    request.key.clone(),
                    (qualified(&request.tool)?, digest(&request.args)?),
                );
                self.acp_options.insert(request.key, request.allow);
                continue;
            }
            if frame["type"] != "control_request" || frame["request"]["subtype"] != "can_use_tool" {
                continue;
            }
            let request = &frame["request"];
            let id = text(&frame, "request_id")
                .or_else(|| text(request, "request_id"))
                .ok_or(Error::Protocol)?;
            let tool = text(request, "tool_name").ok_or(Error::Protocol)?;
            if !tool.starts_with("mcp__") {
                continue;
            }
            if self.requests.len() >= MAX_ENTRIES
                || self.requests.contains_key(&format!("claude:{id}"))
            {
                return Err(Error::Protocol);
            }
            let args = request
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            self.requests
                .insert(format!("claude:{id}"), (qualified(tool)?, digest(&args)?));
        }
        Ok(())
    }
    /// Call only for frames the trusted permission owner writes to its SDK.
    /// The contained worker cannot invoke this direction of the pipe.
    pub fn observe_host_write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > MAX_FRAME {
            return Err(Error::Protocol);
        }
        let frame: Value = serde_json::from_slice(bytes).map_err(|_| Error::Protocol)?;
        if let Some(session) = acp::trusted_session(&frame)? {
            if self.acp_session.as_ref().is_some_and(|old| old != &session) {
                return Err(Error::Protocol);
            }
            self.acp_session = Some(session);
            return Ok(());
        }
        if let Some((id, selected)) = acp::response(&frame)? {
            let Some(binding) = self.requests.remove(&id) else {
                return Ok(());
            };
            let choices = self.acp_options.remove(&id).ok_or(Error::Protocol)?;
            if selected
                .as_ref()
                .is_some_and(|value| choices.contains(value))
            {
                if self.grants.len() >= MAX_ENTRIES {
                    return Err(Error::Busy);
                }
                self.grants.push(binding);
            }
            return Ok(());
        }
        if frame["type"] != "control_response" {
            return Ok(());
        }
        let response = &frame["response"];
        let Some(id) = text(response, "request_id") else {
            return Ok(());
        };
        let Some((tool, original)) = self.requests.remove(&format!("claude:{id}")) else {
            return Ok(());
        };
        if response["subtype"] != "success" || response["response"]["behavior"] != "allow" {
            return Ok(());
        }
        let hash = match response["response"].get("updatedInput") {
            Some(input) => digest(input)?,
            None => original,
        };
        if self.grants.len() >= MAX_ENTRIES {
            return Err(Error::Busy);
        }
        self.grants.push((tool, hash));
        Ok(())
    }
    /// Bind a genuine one-use host grant to a unique MCP RPC identity. A replay
    /// may retrieve the journal result; changed arguments cannot reuse it.
    pub fn permits(
        &mut self,
        turn: &str,
        key: &str,
        qualified: &str,
        args: &Value,
    ) -> Result<bool, Error> {
        if self.turn != turn || key.len() > 512 || key.is_empty() {
            return Ok(false);
        }
        let expected = (self::qualified(qualified)?, digest(args)?);
        if let Some(old) = self.claims.get(key) {
            return Ok(*old == expected);
        }
        if self.claims.len() >= MAX_ENTRIES {
            return Err(Error::Busy);
        }
        let Some(index) = self.grants.iter().position(|g| *g == expected) else {
            return Ok(false);
        };
        self.grants.remove(index);
        self.claims.insert(key.into(), expected);
        Ok(true)
    }
}

#[cfg(test)]
#[path = "provider_mcp_permissions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "provider_mcp_acp_tests.rs"]
mod acp_tests;
