//! Assembly only: the SDK's original provenance is not rewritten or replaced.
use super::composition::ProviderGatewayLease;
use gent_drivers::{ProviderProcess, SupervisorError};
use std::sync::Arc;

fn rejected() -> SupervisorError {
    SupervisorError::Launch("host MCP bridge cannot establish its run authority".into())
}
impl ProviderGatewayLease {
    pub(super) fn prepare_mcp(
        &self,
        launch: &gent_drivers::supervisor::ProviderLaunch,
        control: Option<&crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    ) -> Result<gent_drivers::supervisor::ProviderLaunch, SupervisorError> {
        let mut output = launch.clone();
        if launch.provider != "claude" {
            return Ok(output);
        }
        let matches: Vec<_> = launch
            .arguments
            .iter()
            .enumerate()
            .filter(|(_, v)| *v == "--mcp-config")
            .map(|(i, _)| i)
            .collect();
        if matches.is_empty() {
            return Ok(output);
        }
        if matches.len() != 1 {
            return Err(rejected());
        }
        let index = matches[0] + 1;
        let path = launch.arguments.get(index).ok_or_else(rejected)?;
        let mut held = self.mcp.lock().map_err(|_| rejected())?;
        if held.is_none() {
            #[cfg(target_os = "linux")]
            let deployment = self.mcp_deployment.lock().map_err(|_| rejected())?;
            #[cfg(target_os = "linux")]
            let deployment = deployment.as_ref().ok_or_else(rejected)?;
            let owner = Arc::new(
                super::mcp_owner::McpOwner::prepare(
                    self.fence.clone(),
                    self.broker.client_token(),
                    std::path::Path::new(path),
                    self.private_state.root(),
                    control.ok_or_else(rejected)?,
                    #[cfg(target_os = "linux")]
                    deployment,
                )
                .map_err(|_| rejected())?,
            );
            self.broker
                .attach_mcp(owner.bridge.clone())
                .map_err(|_| rejected())?;
            *held = Some(owner);
        }
        output.arguments[index] = held
            .as_ref()
            .ok_or_else(rejected)?
            .worker_path
            .to_str()
            .ok_or_else(rejected)?
            .into();
        Ok(output)
    }
    #[cfg(windows)]
    pub(crate) fn prepare_claurst_mcp(
        &self,
        source: &std::path::Path,
        contents: &str,
        control: Option<&crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    ) -> Result<(), SupervisorError> {
        use sha2::{Digest, Sha256};
        if self.fence.lock.provider != "claurst" || !self.current() {
            return Err(rejected());
        }
        let expected = format!("{:x}", Sha256::digest(contents.as_bytes()));
        let captured = crate::contained_claude_mcp::HostMcpConfig::capture_claurst(source)
            .map_err(|_| rejected())?;
        if captured.source_sha256() != expected {
            return Err(rejected());
        }
        if captured.servers().is_empty() {
            return Ok(());
        }
        let mut held = self.mcp.lock().map_err(|_| rejected())?;
        if held.is_some() {
            return Err(rejected());
        }
        let owner = Arc::new(
            super::mcp_owner::McpOwner::prepare_claurst(
                self.fence.clone(),
                self.broker.client_token(),
                source,
                &expected,
                self.private_state.root(),
                control.ok_or_else(rejected)?,
            )
            .map_err(|_| rejected())?,
        );
        self.broker
            .attach_mcp(owner.bridge.clone())
            .map_err(|_| rejected())?;
        *held = Some(owner);
        Ok(())
    }
    pub(super) fn publish_mcp<P: ProviderProcess + 'static>(
        &self,
        process: &Arc<P>,
    ) -> Result<(), SupervisorError> {
        let held = self.mcp.lock().map_err(|_| rejected())?;
        if let Some(owner) = held.as_ref() {
            owner.gate.publish(process).map_err(|_| rejected())?;
            owner.activate().map_err(|_| rejected())?;
        }
        Ok(())
    }
    pub(super) fn observe_mcp(
        &self,
        bytes: &[u8],
        from_sdk: bool,
    ) -> Result<(), gent_drivers::interrupt::ProcessTreeError> {
        let bad = || {
            gent_drivers::interrupt::ProcessTreeError::Failed(
                "host MCP permission relay is unavailable".into(),
            )
        };
        let held = self.mcp.lock().map_err(|_| bad())?;
        let Some(owner) = held.as_ref() else {
            return Ok(());
        };
        let Some(turn) = owner.gate.active_turn() else {
            return Ok(());
        };
        let mut relay = owner.gate.relay.lock().map_err(|_| bad())?;
        relay.set_turn(&turn).map_err(|_| bad())?;
        if from_sdk {
            relay.observe_sdk_stdout(bytes)
        } else {
            relay.observe_host_write(bytes)
        }
        .map_err(|_| bad())
    }
}
