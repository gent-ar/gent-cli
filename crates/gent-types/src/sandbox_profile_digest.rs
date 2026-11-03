use super::validation::write_field;
use super::{SandboxLaunchProfile, SandboxNetworkPolicy};
use crate::SandboxResourceGrant;
use sha2::{Digest, Sha256};

impl SandboxLaunchProfile {
    #[must_use]
    pub fn digest_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        if let Some(shell) = &self.shell_runtime {
            write_field(
                &mut hasher,
                "shell-runtime-lock",
                &serde_json::to_string(shell.lock()).unwrap_or_default(),
            );
            if let Some(powershell) = shell.powershell_lock() {
                write_field(
                    &mut hasher,
                    "powershell-runtime-lock",
                    &serde_json::to_string(powershell).unwrap_or_default(),
                );
            }
            if let Some(launcher) = shell.powershell_launcher_lock() {
                write_field(
                    &mut hasher,
                    "powershell-launcher-lock",
                    &serde_json::to_string(launcher).unwrap_or_default(),
                );
            }
            if let Some(egress) = shell.command_egress() {
                write_field(
                    &mut hasher,
                    "command-egress",
                    match egress {
                        super::SandboxCommandEgress::BrokeredInternet => "brokered-internet-v1",
                    },
                );
            }
        }
        if let Some(binding) = &self.permission_binding {
            write_field(
                &mut hasher,
                "permission-binding",
                &binding.digest_material(),
            );
        }
        write_field(
            &mut hasher,
            "workspace",
            self.workspace_root.to_string_lossy().as_ref(),
        );
        for root in &self.readable_roots {
            write_field(&mut hasher, "read", root.to_string_lossy().as_ref());
        }
        for root in &self.writable_roots {
            write_field(&mut hasher, "write", root.to_string_lossy().as_ref());
        }
        for variable in &self.inherited_environment {
            write_field(&mut hasher, "env", variable);
        }
        for resource in &self.resources {
            let role = match resource {
                SandboxResourceGrant::RuntimeReadOnly(_) => "runtime-read",
                SandboxResourceGrant::ConfigurationReadOnly(_) => "config-read",
                SandboxResourceGrant::PrivateStateReadWrite(_) => "private-write",
            };
            write_field(
                &mut hasher,
                role,
                resource.root().to_string_lossy().as_ref(),
            );
        }
        if let Some(gateway) = &self.gateway {
            write_field(&mut hasher, "gateway-api", gateway.api().provider_id());
            write_field(
                &mut hasher,
                "gateway-endpoint",
                &gateway.endpoint().to_string(),
            );
            write_field(&mut hasher, "gateway-token", gateway.scoped_token());
            write_field(&mut hasher, "gateway-peer-nonce", gateway.peer_nonce());
            write_field(
                &mut hasher,
                "gateway-policy",
                gateway.policy_digest_sha256(),
            );
        }
        if let Some(state) = &self.private_state {
            write_field(
                &mut hasher,
                "private-state-layout",
                "v1-home-appdata-localappdata-tmp-claude-codex",
            );
            write_field(&mut hasher, "private-state-run", state.run_id());
            write_field(
                &mut hasher,
                "private-state-root",
                state.root().to_string_lossy().as_ref(),
            );
        }
        match &self.network {
            SandboxNetworkPolicy::Disabled => write_field(&mut hasher, "network", "disabled"),
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256,
            } => write_field(&mut hasher, "egress", policy_digest_sha256),
        }
        write_field(
            &mut hasher,
            "processes",
            &self.limits.max_processes.to_string(),
        );
        write_field(
            &mut hasher,
            "memory",
            &self.limits.max_memory_bytes.to_string(),
        );
        write_field(&mut hasher, "cpu", &self.limits.max_cpu_time_ms.to_string());
        if self.cpu_semantics == super::SandboxCpuSemantics::Uncapped {
            write_field(&mut hasher, "cpu-semantics", "uncapped-worker-v1");
        }
        if let super::SandboxCpuSemantics::MonitoredTotal { poll_ms, wall_ms } = self.cpu_semantics
        {
            write_field(&mut hasher, "cpu-semantics", "monitored-total-v1");
            write_field(&mut hasher, "cpu-poll-ms", &poll_ms.to_string());
            write_field(&mut hasher, "wall-ms", &wall_ms.to_string());
        }
        format!("{:x}", hasher.finalize())
    }
}
