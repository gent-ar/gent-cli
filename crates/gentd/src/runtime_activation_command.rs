use std::path::PathBuf;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub(crate) enum RuntimeCommand {
    Activate {
        #[arg(long)]
        bootstrap_dir: PathBuf,
        #[arg(long)]
        runtime_root: Option<PathBuf>,
        #[arg(long)]
        idle_data_dir: Option<PathBuf>,
        #[arg(long)]
        expected_daemon_sha256: Option<String>,
        #[arg(long)]
        expected_bootstrap_tree_sha256: Option<String>,
        #[arg(long)]
        expected_host_epoch: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    ActivationCapabilities,
    BootstrapIdentity {
        #[arg(long)]
        bootstrap_dir: PathBuf,
    },
}

pub(crate) fn execute(command: RuntimeCommand, data_dir: PathBuf) -> Result<String, String> {
    match command {
        RuntimeCommand::ActivationCapabilities => serde_json::to_string(&serde_json::json!({
            "schema":1, "capabilities":["runtime-activation-lease-v1","runtime-immutable-release-v1"]
        })).map_err(super::display),
        RuntimeCommand::BootstrapIdentity { bootstrap_dir } => serde_json::to_string(&super::bootstrap_identity(&bootstrap_dir)?).map_err(super::display),
        RuntimeCommand::Activate { bootstrap_dir, runtime_root, idle_data_dir, expected_daemon_sha256, expected_bootstrap_tree_sha256, expected_host_epoch, json } => {
            let result = super::activate_expected(bootstrap_dir, runtime_root, idle_data_dir.unwrap_or(data_dir), expected_daemon_sha256.as_deref(), expected_host_epoch, expected_bootstrap_tree_sha256.as_deref())?;
            if json { serde_json::to_string(&result).map_err(super::display) }
            else { Ok(result.daemon_path.display().to_string()) }
        }
    }
}
