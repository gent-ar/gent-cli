//! Combines real OS preflight with separately verified installed authority.
use crate::{
    linux_provider_resources::LinuxProviderResources,
    permission_backend_registry::VerifiedProviderFactoryResources,
};
use gent_types::{RunVersionLock, SandboxResourceGrant};
use std::{path::Path, sync::Arc};
#[path = "linux_provider_probe_core.rs"]
mod core;
pub(crate) use core::ProbeInputs;
impl ProbeInputs for LinuxProviderResources {
    fn recheck(&self) -> Result<(), String> {
        VerifiedProviderFactoryResources::recheck(self)
    }
    fn executable(&self) -> &RunVersionLock {
        self.executable()
    }
    fn helper_lock(&self) -> &RunVersionLock {
        self.helper_lock()
    }
    fn delegated_parent(&self) -> &Path {
        self.delegated_parent()
    }
    fn fingerprint(&self) -> &str {
        self.fingerprint()
    }
    fn grants(&self) -> Vec<SandboxResourceGrant> {
        self.grants()
    }
    fn sdk_version(&self) -> &str {
        self.sdk_version()
    }
}

pub(crate) struct KernelProbeSeal(core::OsProbeWitness);
impl KernelProbeSeal {
    pub(crate) fn matches(&self, r: &LinuxProviderResources) -> bool {
        self.0.matches(r)
    }
}
pub(crate) fn run(resources: Arc<LinuxProviderResources>) -> Result<KernelProbeSeal, String> {
    core::run(resources).map(KernelProbeSeal)
}
#[cfg(test)]
pub(crate) fn run_qa(resources: Arc<dyn ProbeInputs>) -> Result<serde_json::Value, String> {
    core::run(resources).map(|w| w.qa_summary())
}
