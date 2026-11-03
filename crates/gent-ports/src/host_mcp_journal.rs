//! Durable host-side MCP effects. A pending operation is never automatically retried.
use crate::LedgerError;
use gent_types::HostEpoch;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostMcpScope {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub coordinator_id: String,
    pub host_epoch: HostEpoch,
    pub policy_id: String,
    /// Digest of the immutable original host configuration, never its secrets.
    pub source_sha256: String,
    /// A new random generation for this particular bridge owner.
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostMcpAdmission {
    Fresh,
    Completed(Vec<u8>),
    Unknown,
}

pub trait HostMcpJournal: Send + Sync {
    /// Atomically fences epoch, ingress, active turn, policy and run ownership.
    /// The request is hashed; its body is not stored in the ledger.
    fn reserve_host_mcp(
        &self,
        scope: &HostMcpScope,
        key: &str,
        request: &[u8],
    ) -> Result<HostMcpAdmission, LedgerError>;
    /// Stores only a response already checked by the host's secret filter.
    fn complete_host_mcp(
        &self,
        scope: &HostMcpScope,
        key: &str,
        response: &[u8],
    ) -> Result<(), LedgerError>;
    /// May be called after the fence closes; can only make this owner's pending
    /// operation permanently unknown, never publish a result or permit retry.
    fn unknown_host_mcp(&self, scope: &HostMcpScope, key: &str) -> Result<(), LedgerError>;
}
