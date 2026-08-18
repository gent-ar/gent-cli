//! Daemon process assembly. Domain modules remain independent of this boundary.

use crate::compatibility_assessment::CompatibilityAssessment;
#[cfg(unix)]
use crate::private_paths;
use crate::{
    RuntimeFacade, build_runtime_with_update_checks, host_lock, runtime_update_authority,
    runtime_update_bootstrap, runtime_update_config, runtime_update_recovery, startup, transport,
};
use {clap::Parser, std::path::PathBuf};

#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host", version)]
#[allow(clippy::struct_excessive_bools)] // Clap flags are independent authority opt-ins.
struct Args {
    /// Directory containing the local IPC endpoint and durable `SQLite` ledger.
    #[arg(long, env = "GENT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Explicit Unix socket path, primarily for supervised Unix launches and tests.
    #[cfg(unix)]
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Read-only cache of a previously verified signed compatibility manifest.
    #[arg(long, env = "GENT_COMPATIBILITY_CACHE")]
    compatibility_cache: Option<PathBuf>,
    /// Trusted public key as `key-id:lowercase-hex`; may be passed more than once.
    #[arg(long = "compatibility-key", env = "GENT_COMPATIBILITY_KEY")]
    compatibility_keys: Vec<String>,
    /// Durable chat persistence only; never providers, MCP, Git, or the private bridge.
    #[arg(long, env = "GENT_AGENT_CHAT_AUTHORITY")]
    agent_chat_authority: bool,
    /// Serve only a locally cached, revalidated signed runtime-release report.
    /// This does not enable downloads, staging, activation, or self-replacement.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_CHECK_AUTHORITY")]
    runtime_update_check_authority: bool,
    /// Durably plan one already-cached signed runtime release.
    /// This opt-in authority never downloads, stages, health-checks, or replaces this process.
    /// It exists to make an approved external-supervisor handoff auditable before activation is
    /// wired into a later delivery phase.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_PLAN_AUTHORITY")]
    runtime_update_plan_authority: bool,
    /// Confirm a staged successor and open its new writer epoch after binding local IPC.
    /// This is only for an external supervisor after it has verified and started the exact staged
    /// Gent pair. It does not download, stage, replace, or launch another process.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_RECOVER_AUTHORITY")]
    runtime_update_recover_authority: bool,
    #[arg(long, env = "GENT_RUNTIME_UPDATE_ATTEMPT_ID")]
    runtime_update_attempt_id: Option<String>,
    /// Cached signed release metadata required by the explicit read-only check profile.
    #[arg(long, env = "GENT_RUNTIME_RELEASE_CACHE")]
    runtime_release_cache: Option<PathBuf>,
    /// Sigstore-verified public trust document published with runtime release metadata.
    #[arg(long, env = "GENT_RUNTIME_RELEASE_TRUST")]
    runtime_release_trust: Option<PathBuf>,
    /// Trusted release key as `key-id:lowercase-hex`; may be passed more than once.
    #[arg(long = "runtime-release-key", env = "GENT_RUNTIME_RELEASE_KEY")]
    runtime_release_keys: Vec<String>,
    /// Verify staged update metadata and write its cache, then exit without opening a host.
    #[arg(long)]
    verify_runtime_update_material: bool,
    /// Bootstrap file paths, valid only with `--verify-runtime-update-material`.
    #[arg(long)]
    runtime_release_manifest: Option<PathBuf>,
    #[arg(long)]
    runtime_release_archive: Option<PathBuf>,
    #[arg(long)]
    runtime_release_archive_manifest: Option<PathBuf>,
}

pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let _authority_profile = crate::authority_profile::shipped_observer_profile();
    if verify_staged_material(&args)? {
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
    let update_checks = configure_update_checks(&args)?;
    let observed_capabilities = transport::observed_capabilities(
        args.agent_chat_authority,
        update_checks.is_some(),
        args.runtime_update_plan_authority || args.runtime_update_recover_authority,
        false,
    );
    let recovery = run_update_authorities(&args, &data_dir, &observed_capabilities)?;
    let compatibility = CompatibilityAssessment::load(
        args.compatibility_cache.as_deref(),
        &args.compatibility_keys,
        startup::unix_seconds(),
    );
    let runtime = build_runtime_with_update_checks(
        &data_dir,
        &observed_capabilities,
        compatibility,
        update_checks,
    )?;
    serve_local(runtime, &args, &data_dir, recovery.as_ref()).await
}

fn verify_staged_material(args: &Args) -> Result<bool, String> {
    if args.verify_runtime_update_material
        && (args.agent_chat_authority
            || args.runtime_update_check_authority
            || args.runtime_update_plan_authority
            || args.runtime_update_recover_authority
            || !args.runtime_release_keys.is_empty())
    {
        return Err("runtime update bootstrap cannot enable a daemon authority profile".into());
    }
    runtime_update_bootstrap::verify_if_enabled(
        runtime_update_bootstrap::RuntimeUpdateBootstrapConfig {
            enabled: args.verify_runtime_update_material,
            cache_path: args.runtime_release_cache.as_deref(),
            trust_path: args.runtime_release_trust.as_deref(),
            release_path: args.runtime_release_manifest.as_deref(),
            archive_path: args.runtime_release_archive.as_deref(),
            archive_manifest_path: args.runtime_release_archive_manifest.as_deref(),
            now_unix_seconds: startup::unix_seconds(),
        },
    )
}

fn configure_update_checks(
    args: &Args,
) -> Result<Option<runtime_update_config::DaemonRuntimeUpdateChecks>, String> {
    if !args.runtime_update_check_authority
        && !args.runtime_update_plan_authority
        && !args.runtime_update_recover_authority
        && (args.runtime_release_cache.is_some()
            || args.runtime_release_trust.is_some()
            || !args.runtime_release_keys.is_empty()
            || args.runtime_release_manifest.is_some()
            || args.runtime_release_archive.is_some()
            || args.runtime_release_archive_manifest.is_some())
    {
        return Err(
            "runtime release settings require explicit check, plan, or recovery authority".into(),
        );
    }
    let update_checks = args
        .runtime_update_check_authority
        .then(|| {
            runtime_update_config::load(
                true,
                args.runtime_release_cache.as_deref(),
                args.runtime_release_trust.as_deref(),
                &args.runtime_release_keys,
                startup::unix_seconds(),
            )
        })
        .transpose()?
        .flatten();
    Ok(update_checks)
}

fn run_update_authorities(
    args: &Args,
    data_dir: &std::path::Path,
    observed_capabilities: &gent_types::CapabilitySet,
) -> Result<Option<runtime_update_recovery::ConfirmedRuntimeUpdateRecovery>, String> {
    if args.runtime_update_plan_authority && args.runtime_update_recover_authority {
        return Err("runtime update planning and successor recovery are mutually exclusive".into());
    }
    if let Some(record) = runtime_update_authority::plan_if_enabled(
        data_dir,
        observed_capabilities,
        runtime_update_authority::RuntimeUpdatePlanConfig {
            enabled: args.runtime_update_plan_authority,
            attempt_id: args.runtime_update_attempt_id.as_deref(),
            cache_path: args.runtime_release_cache.as_deref(),
            trust_path: args.runtime_release_trust.as_deref(),
            keys: &args.runtime_release_keys,
            now_unix_seconds: startup::unix_seconds(),
        },
    )? {
        eprintln!(
            "planned runtime update {} at {:?}",
            record.attempt_id, record.status.stage
        );
    }
    runtime_update_recovery::confirm_if_enabled(
        data_dir,
        runtime_update_recovery::RuntimeUpdateRecoverConfig {
            enabled: args.runtime_update_recover_authority,
            attempt_id: args.runtime_update_attempt_id.as_deref(),
            cache_path: args.runtime_release_cache.as_deref(),
            trust_path: args.runtime_release_trust.as_deref(),
            keys: &args.runtime_release_keys,
            now_unix_seconds: startup::unix_seconds(),
        },
    )
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
        .unwrap_or_else(|| data_dir.join("gentd.sock"));
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
    crate::transport_windows::serve_named_pipe(&pipe_name(data_dir), runtime).await
}

#[cfg(windows)]
fn pipe_name(data_dir: &std::path::Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", endpoint_hash(data_dir))
}

#[cfg(windows)]
fn endpoint_hash(data_dir: &std::path::Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
