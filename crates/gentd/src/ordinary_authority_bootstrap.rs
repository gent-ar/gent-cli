//! Explicit signed ordinary-provider bootstrap, separate from the observer path.

use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};

use crate::{
    daemon_bootstrap::{self, Args},
    node_runtime_lock::AppNodeRuntimeLock,
    ordinary_authority_composition::compose_ordinary_authority,
    ordinary_authority_release::SignedOrdinaryAuthorityRelease,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
    startup,
};

pub(crate) async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.agent_chat_authority {
        return Err(
            "--agent-chat-authority and --ordinary-authority are mutually exclusive".into(),
        );
    }
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(startup::default_data_dir);
    #[cfg(unix)]
    crate::private_paths::prepare_data_dir(&data_dir)?;
    #[cfg(windows)]
    std::fs::create_dir_all(&data_dir)?;
    let _lock = crate::host_lock::acquire(&data_dir)?;
    let node = AppNodeRuntimeLock::from_environment(&data_dir)?;
    let release_path = args
        .ordinary_authority_release
        .as_deref()
        .ok_or("ordinary authority requires --ordinary-authority-release")?;
    let release = SignedOrdinaryAuthorityRelease::load_bound(
        release_path,
        &keys(&args.ordinary_authority_keys)?,
        &node,
        startup::unix_seconds(),
    )?;
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::TurnFollow,
    ]);
    let state = DaemonCompositionState::open(&data_dir, &profile, release.compatibility())?;
    let authority = compose_ordinary_authority(&state, &release, &node)?;
    let runtime =
        RuntimeFacade::from_state_with_ordinary_terminal_authority(state, None, &authority)?;
    tokio::spawn(async move {
        if let Err(error) = authority.run_cadence().await {
            eprintln!("ordinary lifecycle stopped: {error}");
        }
    });
    daemon_bootstrap::serve_ordinary(runtime, &args, &data_dir).await
}

fn keys(values: &[String]) -> Result<BTreeMap<String, VerifyingKey>, String> {
    if values.is_empty() {
        return Err("ordinary authority requires --ordinary-authority-key".into());
    }
    let mut keys = BTreeMap::new();
    for value in values {
        let (id, encoded) = value
            .split_once(':')
            .ok_or("invalid ordinary authority key")?;
        let bytes = hex::decode(encoded).map_err(|_| "invalid ordinary authority key")?;
        if id.is_empty()
            || encoded.len() != 64
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err("invalid ordinary authority key".into());
        }
        let key = VerifyingKey::from_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| "invalid ordinary authority key")?,
        )
        .map_err(|_| "invalid ordinary authority key")?;
        if keys.insert(id.into(), key).is_some() {
            return Err("duplicate ordinary authority key".into());
        }
    }
    Ok(keys)
}
