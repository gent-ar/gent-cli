mod activity_transport;
#[cfg(test)]
mod activity_transport_tests;
mod agent_chat_api;
mod agent_chat_transport;
#[cfg(test)]
mod agent_chat_transport_tests;
mod api;
mod attachment_api;
mod attachment_transport;
mod compatibility_assessment;
#[cfg(test)]
mod compatibility_lock_tests;
mod conversation_transport;
mod decision_mapping;
mod dependency_actions;
mod dependency_catalog;
#[cfg(test)]
mod dependency_catalog_tests;
mod event_stream;
mod host_lock;
#[cfg(unix)]
mod private_paths;
mod provider_effects;
mod provider_resolver;
#[cfg(test)]
mod provider_resolver_tests;
mod public_runs;
mod runtime_facade;
mod runtime_update_authority;
mod runtime_update_config;
mod runtime_update_recovery;
mod runtime_update_transport;
mod startup;
mod transport;
#[cfg(test)]
mod transport_decision_tests;
#[cfg(test)]
mod transport_event_tests;
#[cfg(test)]
mod transport_stream_tests;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod transport_timeline_tests;
#[cfg(windows)]
mod transport_windows;
#[cfg(all(test, windows))]
mod transport_windows_tests;
use crate::compatibility_assessment::CompatibilityAssessment;
use clap::Parser;
#[cfg(test)]
pub(crate) use runtime_facade::build_runtime;
pub(crate) use runtime_facade::{RuntimeFacade, build_runtime_with_update_checks};
use std::path::PathBuf;
#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host")]
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
    /// Enable only the durable agent-chat create/send/queue ledger profile.
    ///
    /// This is an explicit isolated-authority profile: it never starts a public provider,
    /// MCP server, Git mutation, or private bridge. It is off by default while app cutover is
    /// incomplete.
    #[arg(long, env = "GENT_AGENT_CHAT_AUTHORITY")]
    agent_chat_authority: bool,
    /// Serve only a locally cached, revalidated signed runtime-release report.
    ///
    /// This does not enable downloads, staging, activation, or self-replacement.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_CHECK_AUTHORITY")]
    runtime_update_check_authority: bool,
    /// Durably plan one already-cached signed runtime release.
    ///
    /// This opt-in authority never downloads, stages, health-checks, or replaces this process.
    /// It exists to make an approved external-supervisor handoff auditable before activation is
    /// wired into a later migration phase.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_PLAN_AUTHORITY")]
    runtime_update_plan_authority: bool,
    /// Confirm a staged successor and fence/open its new writer epoch.
    ///
    /// This is only for an external supervisor after it has verified and started the exact staged
    /// Gent pair. It does not download, stage, replace, or launch another process.
    #[arg(long, env = "GENT_RUNTIME_UPDATE_RECOVER_AUTHORITY")]
    runtime_update_recover_authority: bool,
    /// Stable idempotency key for an explicitly approved local planning attempt.
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(startup::default_data_dir);
    #[cfg(unix)]
    private_paths::prepare_data_dir(&data_dir)?;
    #[cfg(windows)]
    std::fs::create_dir_all(&data_dir)?;
    let _host_lock = host_lock::acquire(&data_dir)?;
    if !args.runtime_update_check_authority
        && !args.runtime_update_plan_authority
        && !args.runtime_update_recover_authority
        && (args.runtime_release_cache.is_some()
            || args.runtime_release_trust.is_some()
            || !args.runtime_release_keys.is_empty())
    {
        return Err("runtime release settings require explicit check or plan authority".into());
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
    let observed_capabilities =
        transport::observed_capabilities(args.agent_chat_authority, update_checks.is_some());
    if args.runtime_update_plan_authority && args.runtime_update_recover_authority {
        return Err("runtime update planning and successor recovery are mutually exclusive".into());
    }
    if let Some(record) = runtime_update_authority::plan_if_enabled(
        &data_dir,
        &observed_capabilities,
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
    if let Some(epoch) = runtime_update_recovery::recover_if_enabled(
        &data_dir,
        runtime_update_recovery::RuntimeUpdateRecoverConfig {
            enabled: args.runtime_update_recover_authority,
            attempt_id: args.runtime_update_attempt_id.as_deref(),
            cache_path: args.runtime_release_cache.as_deref(),
            trust_path: args.runtime_release_trust.as_deref(),
            keys: &args.runtime_release_keys,
            now_unix_seconds: startup::unix_seconds(),
        },
    )? {
        eprintln!(
            "confirmed runtime update successor at host epoch {}",
            epoch.0
        );
    }
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
    serve_local(runtime, &args, &data_dir).await
}

#[cfg(unix)]
async fn serve_local(
    runtime: RuntimeFacade,
    args: &Args,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = args
        .socket
        .clone()
        .unwrap_or_else(|| data_dir.join("gentd.sock"));
    transport::serve(private_paths::bind_socket(data_dir, &socket)?, runtime).await
}

#[cfg(windows)]
async fn serve_local(
    runtime: RuntimeFacade,
    _: &Args,
    data_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    transport_windows::serve_named_pipe(&pipe_name(data_dir), runtime).await
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

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
