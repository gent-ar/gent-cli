use std::{fs, path::Path};

use super::{Bootstrap, display, expected_target, stage, valid_version, verify_bootstrap};

pub(super) use gent_protocol::runtime_activation::ActivationBootstrapIdentity as BootstrapIdentity;

pub(super) fn bootstrap_identity(source: &Path) -> Result<BootstrapIdentity, String> {
    let source = source.canonicalize().map_err(display)?;
    verify_bootstrap(&source)?;
    let descriptor: Bootstrap =
        serde_json::from_slice(&fs::read(source.join("bootstrap.json")).map_err(display)?)
            .map_err(display)?;
    if descriptor.target != expected_target()? || !valid_version(&descriptor.version) {
        return Err("runtime bootstrap identity has incompatible metadata".into());
    }
    let tree = stage::scan(&source)?;
    Ok(BootstrapIdentity {
        schema: 1,
        executable_digest_sha256: tree.digest(super::daemon_name())?,
        bootstrap_tree_sha256: tree.fingerprint,
    })
}
