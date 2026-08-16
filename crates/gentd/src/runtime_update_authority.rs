//! Explicit, local-only composition for durable runtime-update planning.
//!
//! It deliberately stops after a verified plan. Archive staging, staged-process health, and
//! activation need an installed bootstrapper handoff with post-exit acknowledgement; they are
//! not safe to infer from cached metadata alone.

use std::path::Path;

use gent_core::RuntimeUpdateContext;
use gent_runtime::{
    Coordinator, RuntimeUpdateAuthority, RuntimeUpdatePlanner, RuntimeUpdatePlanningResult,
    RuntimeUpdateRequest,
};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, RuntimeUpdateRecord};

use crate::runtime_update_config;

/// Plans one trusted cached release only when the non-default authority profile is selected.
///
/// The returned checkpoint is durable and idempotent by `attempt_id`. No network, provider,
/// archive, process, or bootstrapper effect is reached from this composition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuntimeUpdatePlanConfig<'a> {
    pub(crate) enabled: bool,
    pub(crate) attempt_id: Option<&'a str>,
    pub(crate) cache_path: Option<&'a Path>,
    pub(crate) trust_path: Option<&'a Path>,
    pub(crate) keys: &'a [String],
    pub(crate) now_unix_seconds: u64,
}

/// Plans one trusted cached release only when the non-default authority profile is selected.
pub(crate) fn plan_if_enabled(
    data_dir: &Path,
    capabilities: &CapabilitySet,
    config: RuntimeUpdatePlanConfig<'_>,
) -> Result<Option<RuntimeUpdateRecord>, String> {
    if !config.enabled {
        return Ok(None);
    }
    let attempt_id = config
        .attempt_id
        .filter(|value| !value.trim().is_empty())
        .ok_or("runtime update planning requires --runtime-update-attempt-id")?;
    let trusted = runtime_update_config::load_trusted(
        config.cache_path,
        config.trust_path,
        config.keys,
        config.now_unix_seconds,
    )?;
    let ledger = SqliteLedger::open(data_dir.join("gent.db")).map_err(|error| error.to_string())?;
    let coordinator = Coordinator::new(ledger.clone(), capabilities.clone());
    let host_epoch = coordinator
        .status()
        .map_err(|error| error.to_string())?
        .host_epoch;
    let context = RuntimeUpdateContext {
        protocol: trusted.context.protocol,
        schema: trusted.context.schema,
        app_version: trusted.context.app_version,
        selected_channel: trusted.source.release().payload.channel,
        selected_cohort: trusted.context.selected_cohort,
        manifest_verified: false,
        now_unix_seconds: config.now_unix_seconds,
    };
    let planner = RuntimeUpdatePlanner::new(
        coordinator,
        ledger,
        trusted.source,
        trusted.trust,
        RuntimeUpdateAuthority::Approved,
    );
    let request = RuntimeUpdateRequest {
        attempt_id: attempt_id.into(),
        host_epoch,
        target: trusted.context.target,
        context,
    };
    match planner.plan(&request).map_err(|error| error.to_string())? {
        RuntimeUpdatePlanningResult::DeniedObserver => {
            Err("runtime update authority was denied".into())
        }
        RuntimeUpdatePlanningResult::Existing(record) => Ok(Some(record)),
        RuntimeUpdatePlanningResult::Planned(plan) => Ok(Some(plan.record)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signer, SigningKey};
    use gent_ports::{IngressMode, Ledger};
    use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
    use gent_store::SqliteLedger;
    use gent_types::{
        CapabilitySet, RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact,
        RuntimeReleaseChannel, RuntimeReleaseManifest, RuntimeUpdateStage, RuntimeVersion,
        SignedRuntimeRelease,
    };

    use super::{RuntimeUpdatePlanConfig, plan_if_enabled};
    use crate::runtime_update_config::platform_target;

    fn cache(path: &std::path::Path, key: &SigningKey) -> String {
        let payload = RuntimeReleaseManifest {
            manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
            release_version: RuntimeVersion {
                major: 9,
                minor: 0,
                patch: 0,
            },
            protocol_min: 1,
            protocol_max: gent_types::PROTOCOL_MAX,
            schema_min: 1,
            schema_max: gent_store::CURRENT_SCHEMA_VERSION,
            minimum_app_version: RuntimeVersion {
                major: 0,
                minor: 1,
                patch: 0,
            },
            channel: RuntimeReleaseChannel::Stable,
            rollout_percent: 100,
            expires_at_unix_seconds: 100,
            revoked: false,
            forward_only_schema: false,
            artifact: RuntimeReleaseArtifact {
                target: platform_target().unwrap(),
                archive_name: "gent.tar.gz".into(),
                digest_sha256: "a".repeat(64),
                size_bytes: 1,
            },
        };
        let release = SignedRuntimeRelease {
            key_id: "release-1".into(),
            signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
            payload,
        };
        let trust =
            RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
        CachedRuntimeRelease::verify(release, &trust, 1)
            .unwrap()
            .store(path, &trust, 1)
            .unwrap();
        format!("release-1:{}", hex::encode(key.verifying_key().to_bytes()))
    }

    #[test]
    fn disabled_authority_leaves_the_ledger_untouched() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            plan_if_enabled(
                directory.path(),
                &CapabilitySet::default(),
                RuntimeUpdatePlanConfig {
                    enabled: false,
                    attempt_id: None,
                    cache_path: None,
                    trust_path: None,
                    keys: &[],
                    now_unix_seconds: 1,
                }
            )
            .unwrap()
            .is_none()
        );
        assert!(!directory.path().join("gent.db").exists());
    }

    #[test]
    fn approved_authority_plans_cached_release_once_without_external_effects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release.json");
        let key = SigningKey::from_bytes(&[9; 32]);
        let key_text = cache(&path, &key);
        let first = plan_if_enabled(
            directory.path(),
            &CapabilitySet::default(),
            RuntimeUpdatePlanConfig {
                enabled: true,
                attempt_id: Some("local-plan"),
                cache_path: Some(&path),
                trust_path: None,
                keys: &[key_text],
                now_unix_seconds: 1,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.status.stage, RuntimeUpdateStage::Available);
        let second = plan_if_enabled(
            directory.path(),
            &CapabilitySet::default(),
            RuntimeUpdatePlanConfig {
                enabled: true,
                attempt_id: Some("local-plan"),
                cache_path: Some(&path),
                trust_path: None,
                keys: &[],
                now_unix_seconds: 1,
            },
        );
        assert!(second.is_err());
        let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
        assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Open);
    }
}
