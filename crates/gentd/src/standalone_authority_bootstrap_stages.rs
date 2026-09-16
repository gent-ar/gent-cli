use std::path::Path;

use crate::{
    daemon_bootstrap::{self, Args},
    runtime_facade::RuntimeFacade,
    standalone_authority_composition::StandaloneAuthorityRuntime,
    standalone_mcp_config::StandaloneMcpConfig,
};

pub(super) struct StandaloneMcpSettings {
    pub(super) config: Option<StandaloneMcpConfig>,
    pub(super) server_count: u16,
    pub(super) server_names: Vec<String>,
}

pub(super) fn mcp_settings(
    args: &Args,
    data_dir: &Path,
) -> Result<StandaloneMcpSettings, Box<dyn std::error::Error>> {
    let config = args
        .mcp_config
        .as_deref()
        .map(StandaloneMcpConfig::load)
        .transpose()?;
    let config = match config {
        Some(config) => Some(config.with_internal_servers(data_dir)?),
        None => Some(StandaloneMcpConfig::internal_only(data_dir)?),
    };
    let server_count = config
        .as_ref()
        .map(StandaloneMcpConfig::server_count)
        .transpose()?
        .unwrap_or_default();
    let server_names = config
        .as_ref()
        .map(StandaloneMcpConfig::server_names)
        .transpose()?
        .unwrap_or_default();
    Ok(StandaloneMcpSettings {
        config,
        server_count,
        server_names,
    })
}

pub(super) async fn serve_until_stopped(
    authority: StandaloneAuthorityRuntime,
    runtime: RuntimeFacade,
    args: &Args,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let recovered = authority.clone();
    let mut cadence = tokio::spawn(async move { authority.run_cadence().await });
    tokio::select! {
        result = &mut cadence => return lifecycle_stopped(result),
        ready = recovered.wait_until_ready() => ready?,
        () = daemon_bootstrap::terminated() => {
            cadence.abort();
            return Ok(());
        }
    }
    let serve = daemon_bootstrap::serve_ordinary(runtime, args, data_dir);
    tokio::pin!(serve);
    tokio::select! {
        result = &mut cadence => lifecycle_stopped(result),
        result = &mut serve => {
            cadence.abort();
            result
        }
        () = daemon_bootstrap::terminated() => {
            cadence.abort();
            Ok(())
        }
    }
}

fn lifecycle_stopped(
    result: Result<Result<(), String>, tokio::task::JoinError>,
) -> Result<(), Box<dyn std::error::Error>> {
    result.map_err(|_| "standalone provider lifecycle task failed")??;
    Err("standalone provider lifecycle stopped unexpectedly".into())
}
