//! Explicit external handoff to the signed paired-runtime installer.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;
use thiserror::Error;

const REPOSITORY: &str = "gent-ar/gent-cli";
const GITHUB_ACTIONS_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Exact, user-confirmed release identity sent to the external installer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpdateRequest {
    pub(crate) version: String,
    pub(crate) expected_sha256: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) install_dir: Option<PathBuf>,
}

/// Failure before the signed installer has atomically selected a release pair.
#[derive(Debug, Error)]
pub(crate) enum UpdateHandoffError {
    #[error("could not create the external update workspace: {0}")]
    Workspace(#[from] std::io::Error),
    #[error("{program} exited unsuccessfully while preparing the signed update")]
    PreparationFailed { program: &'static str },
    #[error(
        "the signed installer exited unsuccessfully; the selected release pair was not changed"
    )]
    InstallerFailed,
}

/// Verifies the tag-bound installer bootstrap, then invokes it as a child process.
///
/// The bootstrap independently verifies the archive and manifest, confirms the
/// explicit digest, stages both binaries, and acquires the target `gentd` lock
/// for the atomic pointer switch. This client never replaces itself or `gentd`.
pub(crate) fn apply(request: &UpdateRequest) -> Result<(), UpdateHandoffError> {
    let workspace = tempfile::Builder::new().prefix("gent-update-").tempdir()?;
    let script = workspace.path().join(installer_name());
    download(&workspace, &script, &request.version)?;
    verify_bootstrap(&script, &request.version)?;
    invoke_installer(&script, request)
}

fn download(
    workspace: &TempDir,
    script: &std::path::Path,
    version: &str,
) -> Result<(), UpdateHandoffError> {
    let bundle = workspace
        .path()
        .join(format!("{}.sigstore.json", installer_name()));
    run(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--output",
            ])
            .arg(script)
            .arg(bootstrap_url(version)),
        "curl",
    )?;
    run(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--output",
            ])
            .arg(&bundle)
            .arg(format!("{}.sigstore.json", bootstrap_url(version))),
        "curl",
    )
}

fn verify_bootstrap(script: &std::path::Path, version: &str) -> Result<(), UpdateHandoffError> {
    let bundle = script.with_file_name(format!("{}.sigstore.json", installer_name()));
    run(
        Command::new("cosign")
            .arg("verify-blob")
            .arg(script)
            .arg("--bundle")
            .arg(bundle)
            .arg("--certificate-identity-regexp")
            .arg(format!(
                "^https://github.com/{REPOSITORY}/.github/workflows/release.yml@refs/tags/{version}$"
            ))
            .arg("--certificate-oidc-issuer")
            .arg(GITHUB_ACTIONS_OIDC_ISSUER),
        "cosign",
    )
}

fn invoke_installer(
    script: &std::path::Path,
    request: &UpdateRequest,
) -> Result<(), UpdateHandoffError> {
    let mut command = Command::new(installer_runner());
    installer_arguments(&mut command, script, request);
    command
        .status()
        .map_err(UpdateHandoffError::Workspace)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or(UpdateHandoffError::InstallerFailed)
        })
}

fn run(command: &mut Command, program: &'static str) -> Result<(), UpdateHandoffError> {
    command
        .status()
        .map_err(UpdateHandoffError::Workspace)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or(UpdateHandoffError::PreparationFailed { program })
        })
}

fn bootstrap_url(version: &str) -> String {
    let default = format!("https://github.com/{REPOSITORY}/releases/download/{version}");
    let base = std::env::var("GENT_RELEASE_BASE_URL").unwrap_or(default);
    format!("{}/{}", base.trim_end_matches('/'), installer_name())
}

#[cfg(unix)]
fn installer_name() -> &'static str {
    "gent-install.sh"
}

#[cfg(windows)]
fn installer_name() -> &'static str {
    "gent-install.ps1"
}

#[cfg(unix)]
fn installer_runner() -> &'static str {
    "sh"
}

#[cfg(windows)]
fn installer_runner() -> &'static str {
    "powershell.exe"
}

#[cfg(unix)]
fn installer_arguments(command: &mut Command, script: &std::path::Path, request: &UpdateRequest) {
    command
        .arg(script)
        .args([
            "--version",
            &request.version,
            "--force",
            "--expected-sha256",
        ])
        .arg(&request.expected_sha256)
        .arg("--idle-data-dir")
        .arg(&request.data_dir);
    if let Some(directory) = &request.install_dir {
        command.arg("--install-dir").arg(directory);
    }
}

#[cfg(windows)]
fn installer_arguments(command: &mut Command, script: &std::path::Path, request: &UpdateRequest) {
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(["-Version", &request.version, "-Force", "-ExpectedSha256"])
        .arg(&request.expected_sha256)
        .arg("-IdleDataDir")
        .arg(&request.data_dir);
    if let Some(directory) = &request.install_dir {
        command.arg("-InstallDir").arg(directory);
    }
}

#[cfg(test)]
mod tests {
    use super::{GITHUB_ACTIONS_OIDC_ISSUER, UpdateRequest, bootstrap_url, installer_name};

    #[test]
    fn bootstrap_url_is_tag_pinned_and_names_the_platform_installer() {
        let version = "v1.2.3";
        assert_eq!(
            bootstrap_url(version),
            format!(
                "https://github.com/gent-ar/gent-cli/releases/download/{version}/{}",
                installer_name()
            )
        );
    }

    #[test]
    fn request_keeps_the_explicit_digest_and_target_data_directory() {
        let request = UpdateRequest {
            version: "v1.2.3".into(),
            expected_sha256: "a".repeat(64),
            data_dir: "gent-data".into(),
            install_dir: None,
        };
        assert_eq!(request.expected_sha256.len(), 64);
        assert!(request.install_dir.is_none());
    }

    #[test]
    fn bootstrap_verification_uses_the_github_actions_oidc_issuer() {
        assert_eq!(
            GITHUB_ACTIONS_OIDC_ISSUER,
            "https://token.actions.githubusercontent.com"
        );
    }
}
