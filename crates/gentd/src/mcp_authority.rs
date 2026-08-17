//! Dormant, registry-bound MCP authority composition.
//!
//! `main` never constructs this type. A future reviewed composition must pass a profile whose
//! MCP approval pins real evidence and the exact credential-free registry declaration. The
//! injected executor remains the only possible process or network boundary.

use gent_mcp::ToolRegistry;
use gent_ports::{
    Ledger, McpConnectOperation, McpConnectionSummary, McpConnectorError, McpConnectorExecutor,
    McpConnectorLedger, ToolSourceLedger,
};
use gent_runtime::{McpConnectRequest, McpConnectResult, McpConnectorService, RuntimeError};
use sha2::{Digest, Sha256};

use crate::authority_profile::{McpApproval, ValidatedAuthorityProfile};

/// Refuses MCP composition unless the profile and immutable registry agree.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum McpAuthorityError {
    #[error("the observer profile cannot construct MCP authority")]
    ObserverProfile,
    #[error("the approved MCP registry digest does not match the injected registry")]
    RegistryMismatch,
}

/// A composition-only MCP runtime with an injected executor and immutable registry.
///
/// The wrapped runtime service claims receipts and source leases before calling the executor.
/// An unregistered declaration becomes a controlled connector failure and settles its receipt;
/// it cannot reach the injected process or network executor.
pub(crate) struct McpAuthorityRuntime<L, E> {
    connections: McpConnectorService<L, RegistryExecutor<E>>,
}

impl<L, E> McpAuthorityRuntime<L, E>
where
    L: Ledger + McpConnectorLedger + ToolSourceLedger,
    E: McpConnectorExecutor,
{
    /// Binds a reviewed MCP profile to a registry and injected external-effect boundary.
    ///
    /// # Errors
    /// Returns an error before construction when the profile has not prepared MCP authority or
    /// its approved registry digest differs from the supplied pure registry.
    pub(crate) fn new(
        profile: ValidatedAuthorityProfile,
        ledger: L,
        registry: ToolRegistry,
        executor: E,
    ) -> Result<Self, McpAuthorityError> {
        let approval = mcp_approval(profile).ok_or(McpAuthorityError::ObserverProfile)?;
        if registry_sha256(&registry) != approval.registry_sha256 {
            return Err(McpAuthorityError::RegistryMismatch);
        }
        Ok(Self {
            connections: McpConnectorService::new(
                ledger,
                RegistryExecutor { registry, executor },
                true,
            ),
        })
    }

    /// Connects one durable source using the receipt/lease/epoch-fenced runtime service.
    ///
    /// # Errors
    /// Returns an error when durable receipt, lease, source, or connector state cannot be read
    /// or written. Registry misses are settled as controlled connector failures.
    pub(crate) fn connect(
        &self,
        request: &McpConnectRequest,
    ) -> Result<McpConnectResult, RuntimeError> {
        self.connections.connect(request)
    }
}

struct RegistryExecutor<E> {
    registry: ToolRegistry,
    executor: E,
}

impl<E: McpConnectorExecutor> McpConnectorExecutor for RegistryExecutor<E> {
    fn connect(
        &self,
        operation: &McpConnectOperation,
    ) -> Result<McpConnectionSummary, McpConnectorError> {
        if self
            .registry
            .matches_declaration(&operation.source_name, &operation.declared_tools)
        {
            self.executor.connect(operation)
        } else {
            Err(McpConnectorError::Unavailable)
        }
    }
}

pub(crate) fn registry_sha256(registry: &ToolRegistry) -> String {
    let mut hasher = Sha256::new();
    hasher.update(registry.canonical_declaration());
    format!("{:x}", hasher.finalize())
}

fn mcp_approval(profile: ValidatedAuthorityProfile) -> Option<McpApproval> {
    match profile {
        ValidatedAuthorityProfile::PreparedMcp(approval) => Some(approval),
        ValidatedAuthorityProfile::PreparedPublicDriversAndMcp { mcp, .. } => Some(mcp),
        ValidatedAuthorityProfile::Observer
        | ValidatedAuthorityProfile::PreparedPublicDrivers(_) => None,
    }
}
