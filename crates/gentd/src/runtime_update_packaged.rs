use std::path::Path;

use gent_types::{
    RuntimeReleaseChannel, RuntimeUpdateCheckReport, RuntimeUpdateCheckState, RuntimeUpdateFailure,
};

use crate::runtime_update_config::{self, DaemonRuntimeUpdateChecks};

pub(crate) fn current_executable_update_checks(
    now_unix_seconds: u64,
) -> Option<DaemonRuntimeUpdateChecks> {
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()?;
    packaged_update_checks(&executable, now_unix_seconds)
}

pub(crate) fn packaged_update_checks(
    gentd_executable: &Path,
    now_unix_seconds: u64,
) -> Option<DaemonRuntimeUpdateChecks> {
    let release = gentd_executable.parent()?;
    runtime_update_config::load(
        true,
        Some(&release.join("runtime-release-cache.json")),
        Some(&release.join("runtime-release-trust.json")),
        &[],
        now_unix_seconds,
    )
    .ok()
    .flatten()
}

pub(crate) fn metadata_unavailable(channel: RuntimeReleaseChannel) -> RuntimeUpdateCheckReport {
    RuntimeUpdateCheckReport {
        current_version: runtime_update_config::package_version(),
        channel,
        state: RuntimeUpdateCheckState::Unavailable,
        candidate: None,
        failure: Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable),
    }
}

#[cfg(test)]
#[path = "runtime_update_packaged_tests.rs"]
mod tests;
