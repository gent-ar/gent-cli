use std::path::Path;

use crate::{
    daemon_bootstrap::Args, runtime_update_authority, runtime_update_bootstrap,
    runtime_update_config, runtime_update_recovery, startup,
};

pub(super) fn verify_staged_material(args: &Args) -> Result<bool, String> {
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

pub(super) fn configure_update_checks(
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
    args.runtime_update_check_authority
        .then(|| {
            runtime_update_config::load(
                true,
                args.runtime_release_cache.as_deref(),
                args.runtime_release_trust.as_deref(),
                &args.runtime_release_keys,
                startup::unix_seconds(),
            )
        })
        .transpose()
        .map(Option::flatten)
}

pub(super) fn run_update_authorities(
    args: &Args,
    data_dir: &Path,
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
