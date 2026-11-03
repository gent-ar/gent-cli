use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;

use gent_types::AgentChatProvider;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::windows_external_proxy_policy::{ExternalProxyPolicy, PackageIdentity};
use crate::windows_sandbox_runtime::PinnedSandboxHelper;

const DESCRIPTOR_SHA256: Option<&str> = option_env!("GENT_WINDOWS_EXTERNAL_PROXY_POLICY_SHA256");
const RESOURCES_SHA256: Option<&str> = option_env!("GENT_WINDOWS_PROVIDER_RESOURCES_SHA256");

pub(crate) fn require(
    helper: &PinnedSandboxHelper,
    provider: AgentChatProvider,
) -> Result<ExternalProxyPolicy, String> {
    let expected = DESCRIPTOR_SHA256
        .filter(|value| valid_digest(value))
        .ok_or("this daemon has no reviewed external-identity private-network policy descriptor")?;
    let root = helper.runtime_root()?;
    let path = root.join("reviewed-external-proxy-policy.json");
    let canonical = path
        .canonicalize()
        .map_err(|_| "reviewed external proxy policy is missing")?;
    if canonical.parent() != Some(root) {
        return Err("reviewed external proxy policy escaped its runtime directory".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(canonical)
        .map_err(|_| "reviewed external proxy policy cannot be locked")?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "reviewed external proxy policy cannot be read")?;
    if hex::encode(Sha256::digest(&bytes)) != expected {
        return Err("reviewed external proxy policy does not match this daemon".into());
    }
    let policy = ExternalProxyPolicy::decode(&bytes)?;
    policy.require(
        provider,
        &helper.image_lock().digest_sha256,
        RESOURCES_SHA256,
    )?;
    Ok(policy)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Inspection {
    schema: u8,
    nonce: String,
    package: PackageIdentity,
    psec: Psec,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Psec {
    major: u32,
    minor: u32,
    support_flags: u64,
}

pub(crate) fn require_installed_package(
    helper: &PinnedSandboxHelper,
    policy: &ExternalProxyPolicy,
) -> Result<(), String> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let actual: Inspection = crate::windows_helper_inspection::inspect(
        helper,
        "inspect-external-package",
        std::slice::from_ref(&nonce),
    )?;
    validate_inspection(policy, &nonce, &actual)
}

fn validate_inspection(
    policy: &ExternalProxyPolicy,
    nonce: &str,
    actual: &Inspection,
) -> Result<(), String> {
    if actual.schema != 1
        || actual.nonce != nonce
        || actual.psec.major != 1
        || actual.psec.minor < 1
        || actual.psec.support_flags & 9 != 9
    {
        return Err(
            "external proxy package inspection is not current or PSEC is unsupported".into(),
        );
    }
    policy.require_package(&actual.package)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_must_bind_fresh_nonce_exact_package_and_native_support() {
        let policy = ExternalProxyPolicy::decode(
            &serde_json::to_vec(&serde_json::json!({
                "schema":1, "policy":"external-identity-proxy-private-v1", "provider":"codex",
                "ingress":"private-network-capability-accepted", "physicalLanVerified":false,
                "helperSha256":"a".repeat(64), "resourcesSha256":"b".repeat(64),
                "package":{"family":"Gent.Proxy_abc","fullName":"Gent.Proxy_1.0.0.0_x64__abc",
                    "application":"Proxy", "imageSha256":"c".repeat(64),
                    "manifestSha256":"d".repeat(64),"developmentRegistration":true}
            }))
            .unwrap(),
        )
        .unwrap();
        let nonce = "e".repeat(32);
        let mut actual = Inspection {
            schema: 1,
            nonce: nonce.clone(),
            package: policy.package.clone(),
            psec: Psec {
                major: 1,
                minor: 1,
                support_flags: 9,
            },
        };
        assert!(validate_inspection(&policy, &nonce, &actual).is_ok());
        actual.nonce = "f".repeat(32);
        assert!(validate_inspection(&policy, &nonce, &actual).is_err());
        actual.nonce = nonce.clone();
        for flags in [0, 1, 8] {
            actual.psec.support_flags = flags;
            assert!(validate_inspection(&policy, &nonce, &actual).is_err());
        }
        actual.psec.support_flags = 9;
        actual.psec.minor = 0;
        assert!(validate_inspection(&policy, &nonce, &actual).is_err());
        actual.psec.minor = 1;
        actual.psec.major = 2;
        assert!(validate_inspection(&policy, &nonce, &actual).is_err());
        actual.psec.major = 1;
        actual.package.development_registration = false;
        assert!(validate_inspection(&policy, &nonce, &actual).is_err());
    }
}
