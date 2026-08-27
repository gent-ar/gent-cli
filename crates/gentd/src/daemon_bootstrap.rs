//! Daemon process assembly. Domain modules remain independent of this boundary.

use crate::compatibility_assessment::CompatibilityAssessment;
#[cfg(unix)]
use crate::private_paths;
use crate::{
    RuntimeFacade, build_runtime_with_update_checks, host_lock, runtime_update_recovery, startup,
    transport,
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use {
    clap::{Parser, ValueEnum},
    std::path::PathBuf,
};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderLogin {
    Claude,
    Codex,
}

#[path = "daemon_bootstrap_updates.rs"]
mod updates;

#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host", version)]
pub(crate) struct Args {
    /// Directory containing the local IPC endpoint and durable `SQLite` ledger.
    #[arg(long, env = "GENT_DATA_DIR")]
    pub(crate) data_dir: Option<PathBuf>,
    /// Print the resolved data directory and exit without binding IPC or acquiring the host lock.
    ///
    /// Any host that launches the packaged `gentd` runtime (including a native application)
    /// should resolve the shared data directory through this flag rather than duplicating the
    /// `GENT_DATA_DIR` and platform-default resolution rules.
    #[arg(long)]
    pub(crate) print_data_dir: bool,
    /// Explicit Unix socket path, primarily for supervised Unix launches and tests.
    #[cfg(unix)]
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Read-only cache of a previously verified signed compatibility manifest.
    #[arg(long, env = "GENT_COMPATIBILITY_CACHE")]
    pub(crate) compatibility_cache: Option<PathBuf>,
    /// Trusted public key as `key-id:lowercase-hex`; may be passed more than once.
    #[arg(long = "compatibility-key", env = "GENT_COMPATIBILITY_KEY")]
    pub(crate) compatibility_keys: Vec<String>,
    /// Durable chat persistence only; never providers, MCP, Git, or the private bridge.
    #[arg(long, env = "GENT_AGENT_CHAT_AUTHORITY")]
    pub(crate) agent_chat_authority: bool,
    #[arg(long, env = "GENT_STANDALONE_AUTHORITY")]
    pub(crate) standalone_authority: bool,
    #[arg(long, env = "GENT_CLAUDE_EXECUTABLE")]
    pub(crate) standalone_claude_executable: Option<PathBuf>,
    #[arg(long, env = "GENT_CODEX_EXECUTABLE")]
    pub(crate) standalone_codex_executable: Option<PathBuf>,
    #[arg(long, env = "GENT_CLAURST_EXECUTABLE")]
    pub(crate) standalone_claurst_executable: Option<PathBuf>,
    #[arg(long, env = "GENT_LLAMA_SERVER_EXECUTABLE")]
    pub(crate) standalone_llama_server_executable: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub(crate) provider_login: Option<ProviderLogin>,
    #[arg(long, env = "GENT_MCP_CONFIG_PATH")]
    pub(crate) mcp_config: Option<PathBuf>,
    /// Serve only a locally cached, revalidated signed runtime-release report.
    /// This does not enable downloads, staging, activation, or self-replacement.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_CHECK_AUTHORITY")]
    pub(crate) runtime_update_check_authority: bool,
    /// Durably plan one already-cached signed runtime release.
    /// This opt-in authority never downloads, stages, health-checks, or replaces this process.
    /// It exists to make an approved external-supervisor handoff auditable before activation is
    /// wired into a later delivery phase.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_PLAN_AUTHORITY")]
    pub(crate) runtime_update_plan_authority: bool,
    /// Confirm a staged successor and open its new writer epoch after binding local IPC.
    /// This is only for an external supervisor after it has verified and started the exact staged
    /// Gent pair. It does not download, stage, replace, or launch another process.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_RECOVER_AUTHORITY")]
    pub(crate) runtime_update_recover_authority: bool,
    #[arg(long, env = "GENT_RUNTIME_UPDATE_ATTEMPT_ID")]
    pub(crate) runtime_update_attempt_id: Option<String>,
    /// Cached signed release metadata required by the explicit read-only check profile.
    #[arg(long, env = "GENT_RUNTIME_RELEASE_CACHE")]
    pub(crate) runtime_release_cache: Option<PathBuf>,
    /// Sigstore-verified public trust document published with runtime release metadata.
    #[arg(long, env = "GENT_RUNTIME_RELEASE_TRUST")]
    pub(crate) runtime_release_trust: Option<PathBuf>,
    /// Trusted release key as `key-id:lowercase-hex`; may be passed more than once.
    #[arg(long = "runtime-release-key", env = "GENT_RUNTIME_RELEASE_KEY")]
    pub(crate) runtime_release_keys: Vec<String>,
    /// Verify staged update metadata and write its cache, then exit without opening a host.
    #[arg(long)]
    pub(crate) verify_runtime_update_material: bool,
    /// Bootstrap file paths, valid only with `--verify-runtime-update-material`.
    #[arg(long)]
    pub(crate) runtime_release_manifest: Option<PathBuf>,
    #[arg(long)]
    pub(crate) runtime_release_archive: Option<PathBuf>,
    #[arg(long)]
    pub(crate) runtime_release_archive_manifest: Option<PathBuf>,
}

pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.data_dir.is_none() {
        gent_types::migrate_legacy_default_data_dir()
            .map_err(|error| format!("could not migrate the legacy .gent-cli data directory: {error}"))?;
    }
    if args.print_data_dir {
        let data_dir = args
            .data_dir
            .clone()
            .unwrap_or_else(startup::default_data_dir);
        println!("{}", data_dir.display());
        return Ok(());
    }
    if let Some(provider) = args.provider_login {
        let data_dir = args
            .data_dir
            .as_deref()
            .map_or_else(startup::default_data_dir, PathBuf::from);
        return crate::standalone_provider_setup::login(&data_dir, provider).map_err(Into::into);
    }
    if args.standalone_authority {
        return crate::standalone_authority_bootstrap::run(args).await;
    }
    enforce_hard_observer(&crate::authority_profile::shipped_observer_profile())?;
    if updates::verify_staged_material(&args)? {
        return Ok(());
    }
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(startup::default_data_dir);
    #[cfg(unix)]
    private_paths::prepare_data_dir(&data_dir)?;
    #[cfg(windows)]
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    let update_checks = updates::configure_update_checks(&args)?;
    let capability_profile = RuntimeCapabilityProfile::new(
        [
            args.agent_chat_authority
                .then_some(RuntimeCapabilityFeature::AgentChat),
            update_checks
                .is_some()
                .then_some(RuntimeCapabilityFeature::RuntimeUpdateCheck),
            (args.runtime_update_plan_authority || args.runtime_update_recover_authority)
                .then_some(RuntimeCapabilityFeature::RuntimeMaintenance),
        ]
        .into_iter()
        .flatten(),
    );
    let observed_capabilities = transport::observed_capabilities(&capability_profile);
    let recovery = updates::run_update_authorities(&args, &data_dir, &observed_capabilities)?;
    let compatibility = CompatibilityAssessment::load(
        args.compatibility_cache.as_deref(),
        &args.compatibility_keys,
        startup::unix_seconds(),
    );
    let runtime = build_runtime_with_update_checks(
        &data_dir,
        &capability_profile,
        compatibility,
        update_checks,
    )?;
    serve_local(runtime, &args, &data_dir, recovery.as_ref()).await
}

/// Refuses startup if a future bootstrap edit attempts to select a non-observer authority profile.
///
/// The shipped argument surface only produces `Observer`; lifecycle authority must instead be
/// added through a separately reviewed composition and capability milestone.
fn enforce_hard_observer(
    profile: &crate::authority_profile::ValidatedAuthorityProfile,
) -> Result<(), String> {
    profile
        .is_hard_observer()
        .then_some(())
        .ok_or_else(|| "gentd bootstrap only supports the hard observer profile".into())
}

#[cfg(unix)]
async fn serve_local(
    runtime: RuntimeFacade,
    args: &Args,
    data_dir: &std::path::Path,
    recovery: Option<&runtime_update_recovery::ConfirmedRuntimeUpdateRecovery>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = args
        .socket
        .clone()
        .unwrap_or_else(|| gent_types::local_socket_path(data_dir));
    let listener = private_paths::bind_socket(data_dir, &socket)?;
    if let Some(recovery) = recovery {
        let epoch = runtime_update_recovery::open_confirmed(data_dir, recovery)?;
        eprintln!(
            "opened confirmed runtime update successor at host epoch {}",
            epoch.0
        );
    }
    transport::serve(listener, runtime).await
}

pub(crate) async fn serve_ordinary(
    runtime: RuntimeFacade,
    args: &Args,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    serve_local(runtime, args, data_dir, None).await
}

#[cfg(windows)]
async fn serve_local(
    runtime: RuntimeFacade,
    _: &Args,
    data_dir: &std::path::Path,
    recovery: Option<&runtime_update_recovery::ConfirmedRuntimeUpdateRecovery>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(recovery) = recovery {
        let epoch = runtime_update_recovery::open_confirmed(data_dir, recovery)?;
        eprintln!(
            "opened confirmed runtime update successor at host epoch {}",
            epoch.0
        );
    }
    crate::transport_windows::serve_named_pipe(&gent_types::windows_pipe_name(data_dir), runtime)
        .await
}

#[cfg(test)]
#[path = "daemon_bootstrap_tests.rs"]
mod tests;
