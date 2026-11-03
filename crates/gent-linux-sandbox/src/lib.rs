//! Atomic Linux provider containment using pinned typed MXC and owned cgroup v2.
#![cfg(target_os = "linux")]
mod immutable_image;
/// Verify an installed helper identity without activation or OS evidence.
pub fn verify_helper_installation(lock: &gent_types::RunVersionLock) -> Result<(), &'static str> {
    if lock.provider != "gent-linux-sandbox-helper" {
        return Err("helper purpose mismatch");
    }
    immutable_image::require(std::path::Path::new(&lock.canonical_path))?;
    if gent_drivers::lock::rechecked_identity(lock).map_err(|_| "helper identity unavailable")?
        != *lock
    {
        return Err("helper changed");
    }
    Ok(())
}
mod translation;
pub(crate) use translation::translate;
#[doc(hidden)]
pub mod claude_environment;
#[doc(hidden)]
pub mod contained_gate;
pub mod delegation;
pub mod gateway_bridge;
#[doc(hidden)]
pub mod linux_cgroup_contract;
pub mod mxc_baseline;
pub mod native_runtime;
pub mod network_runtime;
#[doc(hidden)]
pub mod sdk_launch;
pub use native_runtime::{
    GatewayAuthority, LinuxMxcLauncher, LinuxMxcProcess, ProbeExecutionWitness,
};
pub use network_runtime::RuntimePin;

/// Read-only preflight of the exact provider image, explicit profile and launch tuple.
/// Successful preflight does not issue OS evidence or create a process.
pub fn validate_launch(
    request: &gent_types::SandboxedLaunchRequest,
    launch: &gent_drivers::supervisor::ProviderLaunch,
) -> Result<(), gent_drivers::sandboxed_launch::SandboxedProviderLaunchError> {
    mxc_baseline::validate(&request.profile).map_err(|_| {
        gent_drivers::sandboxed_launch::SandboxedProviderLaunchError::ProfileRejected
    })?;
    translate(request, launch).map(|_| ())
}

pub mod host_probe;

pub mod sealed_exec;

#[cfg(target_os = "linux")]
pub mod host_stdio;
#[cfg(target_os = "linux")]
mod node_entry;
#[cfg(target_os = "linux")]
pub mod node_profile;
