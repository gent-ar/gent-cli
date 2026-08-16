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
pub(crate) use runtime_facade::{RuntimeFacade, build_runtime};
use std::path::PathBuf;
#[derive(Debug, Parser)]
#[command(name = "gentd", about = "Gent's local runtime host")]
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
    let observed_capabilities = transport::observed_capabilities(args.agent_chat_authority);
    let compatibility = CompatibilityAssessment::load(
        args.compatibility_cache.as_deref(),
        &args.compatibility_keys,
        startup::unix_seconds(),
    );
    let runtime = build_runtime(&data_dir, &observed_capabilities, compatibility)?;
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
