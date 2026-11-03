//! Signed installation authority, never live containment or credential evidence.
//! No executable is discovered on PATH and no unverified descriptor enables modes.
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_DESCRIPTOR: u64 = 65536;
fn platform() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some("x86_64-unknown-linux-gnu");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Some("aarch64-unknown-linux-gnu");
    #[allow(unreachable_code)]
    None
}
const MXC: &str = "fda29ac63bc306b7f97f3b86b4b63d44f7f40cf2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FilePin {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NetworkPin {
    pub path: String,
    pub manifest_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Payload {
    schema: u8,
    authority_release_sha256: String,
    expires_at_unix_seconds: u64,
    revoked: bool,
    platform: String,
    mxc_revision: String,
    baseline_policy: String,
    cpu_policy: String,
    helper: FilePin,
    node: FilePin,
    gent: FilePin,
    network: NetworkPin,
    mcp_entries: BTreeMap<String, FilePin>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Signed {
    key_id: String,
    payload: Payload,
    signature_hex: String,
}
#[derive(Clone, Debug)]
pub(crate) struct VerifiedLinuxDeployment {
    root: PathBuf,
    descriptor: PathBuf,
    descriptor_sha256: String,
    payload: Payload,
}
fn digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn relative(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 1024
        && v.is_ascii()
        && !v.contains('\\')
        && Path::new(v)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && v.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn finite(p: &Payload, ordinary: &str, now: u64) -> bool {
    p.schema == 1
        && !p.revoked
        && p.expires_at_unix_seconds > now
        && digest(ordinary)
        && p.authority_release_sha256 == ordinary
        && Some(p.platform.as_str()) == platform()
        && p.mxc_revision == MXC
        && p.baseline_policy == "mxc-1.0.1-native-readonly"
        && p.cpu_policy == "monitored-total-5ms"
        && p.mcp_entries
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == BTreeSet::from(["gent-browser", "gent-canvas", "mate-canvas"])
        && relative(&p.network.path)
        && digest(&p.network.manifest_sha256)
        && [
            (&p.helper, 64 * 1024 * 1024),
            (&p.node, 512 * 1024 * 1024),
            (&p.gent, 128 * 1024 * 1024),
        ]
        .into_iter()
        .all(|(f, max)| relative(&f.path) && digest(&f.sha256) && (64..=max).contains(&f.bytes))
        && p.mcp_entries.values().all(|f| {
            relative(&f.path) && digest(&f.sha256) && (64..=2 * 1024 * 1024).contains(&f.bytes)
        })
        && {
            let mut paths = BTreeSet::new();
            [&p.helper, &p.node, &p.gent]
                .into_iter()
                .chain(p.mcp_entries.values())
                .all(|f| paths.insert(&f.path))
        }
}
fn verify(
    bytes: &[u8],
    keys: &BTreeMap<String, VerifyingKey>,
    ordinary: &str,
    now: u64,
) -> Result<Payload, &'static str> {
    if bytes.is_empty()
        || bytes.len() > usize::try_from(MAX_DESCRIPTOR).unwrap_or(usize::MAX)
        || keys.is_empty()
        || keys.len() > 8
    {
        return Err("Linux deployment descriptor bounds");
    }
    let signed: Signed =
        serde_json::from_slice(bytes).map_err(|_| "Linux deployment descriptor malformed")?;
    if !finite(&signed.payload, ordinary, now) {
        return Err("Linux deployment contract unavailable");
    }
    let key = keys
        .get(&signed.key_id)
        .ok_or("Linux deployment signing root unknown")?;
    let signature = hex::decode(&signed.signature_hex)
        .ok()
        .and_then(|v| Signature::from_slice(&v).ok())
        .ok_or("Linux deployment signature malformed")?;
    let canonical =
        serde_json::to_vec(&signed.payload).map_err(|_| "Linux deployment canonical encoding")?;
    key.verify(&canonical, &signature)
        .map_err(|_| "Linux deployment signature rejected")?;
    Ok(signed.payload)
}
fn under(root: &Path, relative_path: &str) -> Result<PathBuf, &'static str> {
    if !relative(relative_path) {
        return Err("Linux deployment relative path");
    }
    let target = root.join(relative_path);
    for part in target.ancestors().take_while(|p| p.starts_with(root)) {
        if part
            .symlink_metadata()
            .map_err(|_| "Linux deployment file unavailable")?
            .file_type()
            .is_symlink()
        {
            return Err("Linux deployment alias");
        }
    }
    if target.canonicalize().map_err(|_| "Linux deployment path")? != target {
        return Err("Linux deployment alias");
    }
    Ok(target)
}
// Call only after the daemon-owned AppNodeRuntimeLock has been rechecked.
// Payload/worker/environment fields never choose this resource root.
pub(crate) fn resource_root_for_node(node: &Path) -> Result<PathBuf, &'static str> {
    if !node.is_absolute()
        || node
            .canonicalize()
            .map_err(|_| "Linux packaged Node path")?
            != node
        || !node.is_file()
    {
        return Err("Linux packaged Node path");
    }
    let root = node
        .ancestors()
        .nth(4)
        .ok_or("Linux packaged release root")?;
    if node != root.join("runtime/node/bin/node")
        || root
            .canonicalize()
            .map_err(|_| "Linux packaged release root")?
            != root
    {
        return Err("Linux Node does not belong to the packaged release layout");
    }
    Ok(root.into())
}
fn checked_descriptor_root(path: &Path, resource_root: &Path) -> Result<PathBuf, &'static str> {
    if !resource_root.is_absolute()
        || resource_root
            .canonicalize()
            .map_err(|_| "Linux resource root")?
            != resource_root
        || !resource_root.is_dir()
    {
        return Err("Linux resource root");
    }
    if path != resource_root.join("authority/linux-sandbox.json")
        || !path.is_absolute()
        || path.canonicalize().map_err(|_| "Linux deployment absent")? != path
    {
        return Err("Linux companion is outside the verified release layout");
    }
    Ok(resource_root.into())
}
impl VerifiedLinuxDeployment {
    pub(crate) fn fingerprint(&self) -> &str {
        &self.descriptor_sha256
    }
    /// `keys` must come from the already-configured ordinary release authority,
    /// never from fields inside this companion or a worker/environment override.
    pub(crate) fn load(
        path: &Path,
        resource_root: &Path,
        keys: &BTreeMap<String, VerifyingKey>,
        ordinary: &str,
        now: u64,
    ) -> Result<Self, &'static str> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let root = checked_descriptor_root(path, resource_root)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
            .open(path)
            .map_err(|_| "Linux deployment absent")?;
        let before = file.metadata().map_err(|_| "Linux deployment metadata")?;
        if !before.is_file() || before.len() > MAX_DESCRIPTOR {
            return Err("Linux deployment descriptor bounds");
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_DESCRIPTOR + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Linux deployment read")?;
        let payload = verify(&bytes, keys, ordinary, now)?;
        let after = file
            .metadata()
            .map_err(|_| "Linux deployment handle changed")?;
        let current = path
            .metadata()
            .map_err(|_| "Linux deployment path changed")?;
        let stamp = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if stamp(&before) != stamp(&after) || stamp(&before) != stamp(&current) {
            return Err("Linux deployment changed during verification");
        }
        let result = Self {
            root,
            descriptor: path.into(),
            descriptor_sha256: format!("{:x}", Sha256::digest(bytes)),
            payload,
        };
        result.recheck_files()?;
        Ok(result)
    }
    pub(crate) fn helper(&self) -> &FilePin {
        &self.payload.helper
    }
    pub(crate) fn node(&self) -> &FilePin {
        &self.payload.node
    }
    pub(crate) fn gent(&self) -> &FilePin {
        &self.payload.gent
    }
    pub(crate) fn entries(&self) -> &BTreeMap<String, FilePin> {
        &self.payload.mcp_entries
    }
    pub(crate) fn file_path(&self, pin: &FilePin) -> Result<PathBuf, &'static str> {
        under(&self.root, &pin.path)
    }
    pub(crate) fn network(&self) -> Result<gent_linux_sandbox::RuntimePin, &'static str> {
        let pin = gent_linux_sandbox::RuntimePin {
            root: under(&self.root, &self.payload.network.path)?
                .to_str()
                .ok_or("Linux network runtime path")?
                .into(),
            manifest_sha256: self.payload.network.manifest_sha256.clone(),
        };
        pin.verify()?;
        Ok(pin)
    }
    pub(crate) fn recheck_files(&self) -> Result<(), &'static str> {
        // Signature expiry/revocation must be rechecked by the owning release object
        // before each admission; this method only pins the exact file generations.
        let mut descriptor = Vec::new();
        File::open(&self.descriptor)
            .map_err(|_| "Linux descriptor changed")?
            .take(MAX_DESCRIPTOR + 1)
            .read_to_end(&mut descriptor)
            .map_err(|_| "Linux descriptor changed")?;
        if descriptor.len() > usize::try_from(MAX_DESCRIPTOR).unwrap_or(usize::MAX)
            || format!("{:x}", Sha256::digest(descriptor)) != self.descriptor_sha256
        {
            return Err("Linux descriptor changed");
        }
        for pin in [&self.payload.helper, &self.payload.node, &self.payload.gent]
            .into_iter()
            .chain(self.payload.mcp_entries.values())
        {
            let path = self.file_path(pin)?;
            let mut file = File::open(&path).map_err(|_| "Linux resource absent")?;
            if file
                .metadata()
                .map_err(|_| "Linux resource metadata")?
                .len()
                != pin.bytes
            {
                return Err("Linux resource size changed");
            }
            let mut digest = Sha256::new();
            let mut buffer = vec![0; 65536];
            let mut total = 0u64;
            loop {
                let n = file.read(&mut buffer).map_err(|_| "Linux resource read")?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > pin.bytes {
                    return Err("Linux resource grew");
                }
                digest.update(&buffer[..n]);
            }
            if total != pin.bytes || format!("{:x}", digest.finalize()) != pin.sha256 {
                return Err("Linux resource digest changed");
            }
        }
        self.network()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    pub(super) fn payload() -> Payload {
        let file = |path: &str| FilePin {
            path: path.into(),
            sha256: "a".repeat(64),
            bytes: 128,
        };
        Payload {
            schema: 1,
            authority_release_sha256: "b".repeat(64),
            expires_at_unix_seconds: 200,
            revoked: false,
            platform: platform().unwrap().into(),
            mxc_revision: MXC.into(),
            baseline_policy: "mxc-1.0.1-native-readonly".into(),
            cpu_policy: "monitored-total-5ms".into(),
            helper: file("bin/helper"),
            node: file("node/bin/node"),
            gent: file("bin/gent"),
            network: NetworkPin {
                path: "network".into(),
                manifest_sha256: "c".repeat(64),
            },
            mcp_entries: ["gent-browser", "gent-canvas", "mate-canvas"]
                .into_iter()
                .map(|name| (name.into(), file(&format!("mcp/{name}.js"))))
                .collect(),
        }
    }
    pub(super) fn signed(p: Payload) -> (Vec<u8>, BTreeMap<String, VerifyingKey>) {
        let key = SigningKey::from_bytes(&[0x4a; 32]);
        let bytes = serde_json::to_vec(&p).unwrap();
        let envelope = Signed {
            key_id: "isolated-qa-only".into(),
            payload: p,
            signature_hex: hex::encode(key.sign(&bytes).to_bytes()),
        };
        (
            serde_json::to_vec(&envelope).unwrap(),
            BTreeMap::from([("isolated-qa-only".into(), key.verifying_key())]),
        )
    }
    #[test]
    fn actual_signature_and_release_binding() {
        let (bytes, keys) = signed(payload());
        assert!(verify(&bytes, &keys, &"b".repeat(64), 100).is_ok());
        assert!(verify(&bytes, &keys, &"d".repeat(64), 100).is_err());
        let mut altered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        altered["payload"]["helper"]["sha256"] = serde_json::json!("e".repeat(64));
        assert!(
            verify(
                &serde_json::to_vec(&altered).unwrap(),
                &keys,
                &"b".repeat(64),
                100
            )
            .is_err()
        );
    }
    #[test]
    fn expiration_revocation_cpu_and_full_bundle_set_fail_closed() {
        for change in [
            "expiry",
            "revoked",
            "hard-cpu",
            "missing-entry",
            "extra-entry",
            "alias",
            "oversize",
        ] {
            let mut p = payload();
            match change {
                "expiry" => p.expires_at_unix_seconds = 100,
                "revoked" => p.revoked = true,
                "hard-cpu" => p.cpu_policy = "hard-total".into(),
                "missing-entry" => {
                    p.mcp_entries.remove("mate-canvas");
                }
                "extra-entry" => {
                    p.mcp_entries
                        .insert("arbitrary-package".into(), p.helper.clone());
                }
                "alias" => p.helper.path = "../helper".into(),
                _ => p.mcp_entries.get_mut("gent-browser").unwrap().bytes = 2 * 1024 * 1024 + 1,
            }
            let (bytes, keys) = signed(p);
            assert!(
                verify(&bytes, &keys, &"b".repeat(64), 100).is_err(),
                "{change}"
            );
        }
    }
    #[test]
    fn unknown_signer_unknown_field_and_descriptor_quota_fail_closed() {
        let (bytes, keys) = signed(payload());
        assert!(verify(&bytes, &BTreeMap::new(), &"b".repeat(64), 100).is_err());
        let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        v["payload"]["allowUnreviewed"] = serde_json::json!(true);
        assert!(
            verify(
                &serde_json::to_vec(&v).unwrap(),
                &keys,
                &"b".repeat(64),
                100
            )
            .is_err()
        );
        assert!(verify(&vec![b' '; 65537], &keys, &"b".repeat(64), 100).is_err());
    }
}

#[cfg(test)]
#[path = "linux_provider_deployment_layout_tests.rs"]
mod layout_tests;
