#[derive(Debug)]
pub struct SandboxNetworkPeer {
    identity: String,
    port: u16,
    process_id: u32,
    creation_time: u64,
    kind: PeerKind,
}

#[derive(Debug)]
enum PeerKind {
    LegacyProfile,
    #[cfg(target_os = "linux")]
    LinuxOwnedProxy {
        nonce: String,
    },
    ExternalPackage {
        full_name: String,
        nonce: String,
    },
}

fn nonce(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn package_identity(family: &str, full_name: &str) -> bool {
    let Some((name, publisher)) = family.rsplit_once('_') else {
        return false;
    };
    let plain = |value: &str| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    };
    if family.len() > 255
        || full_name.len() > 255
        || name.is_empty()
        || !plain(name)
        || publisher.len() != 13
        || !publisher
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return false;
    }
    let fields = full_name.split('_').collect::<Vec<_>>();
    if fields.len() != 5
        || fields[0] != name
        || fields[4] != publisher
        || !matches!(fields[2], "x64" | "x86" | "arm64" | "arm" | "neutral")
        || !plain(fields[3])
    {
        return false;
    }
    let version = fields[1].split('.').collect::<Vec<_>>();
    version.len() == 4
        && version.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u16>().is_ok()
        })
}

impl SandboxNetworkPeer {
    pub fn new(identity: String, port: u16, process_id: u32, creation_time: u64) -> Option<Self> {
        let suffix = identity.strip_prefix("GentGateway-")?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || port == 0
            || process_id == 0
            || creation_time == 0
        {
            return None;
        }
        Some(Self {
            identity,
            port,
            process_id,
            creation_time,
            kind: PeerKind::LegacyProfile,
        })
    }

    /// Native package proof is distinct from an old per-run `AppContainer` profile.
    /// The pinned native helper validates the installed package and retained kernel
    /// process/image handles before emitting this public, non-bearer evidence.
    pub fn external_package(
        family: String,
        full_name: String,
        nonce_value: String,
        port: u16,
        process_id: u32,
        creation_time: u64,
    ) -> Option<Self> {
        if !package_identity(&family, &full_name)
            || !nonce(&nonce_value)
            || port == 0
            || process_id == 0
            || creation_time == 0
        {
            return None;
        }
        Some(Self {
            identity: family,
            port,
            process_id,
            creation_time,
            kind: PeerKind::ExternalPackage {
                full_name,
                nonce: nonce_value,
            },
        })
    }

    /// Linux-only owned loopback bridge. Creation time is /proc starttime ticks,
    /// not FILETIME; the backend retains its cgroup/process/namespace handles.
    #[cfg(target_os = "linux")]
    pub fn linux_owned_proxy(
        profile_digest: String,
        nonce_value: String,
        port: u16,
        process_id: u32,
        creation_time: u64,
    ) -> Option<Self> {
        if profile_digest.len() != 64
            || !profile_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !nonce(&nonce_value)
            || port == 0
            || process_id == 0
            || creation_time == 0
        {
            return None;
        }
        Some(Self {
            identity: format!("GentLinuxGateway-{profile_digest}"),
            port,
            process_id,
            creation_time,
            kind: PeerKind::LinuxOwnedProxy { nonce: nonce_value },
        })
    }
    pub fn matches_active_gateway(&self, expected_nonce: &str) -> bool {
        match &self.kind {
            PeerKind::ExternalPackage { nonce, .. } => nonce == expected_nonce,
            #[cfg(target_os = "linux")]
            PeerKind::LinuxOwnedProxy { nonce } => nonce == expected_nonce,
            PeerKind::LegacyProfile => false,
        }
    }

    pub fn legacy_profile_identity(&self) -> Option<&str> {
        matches!(self.kind, PeerKind::LegacyProfile).then_some(self.identity.as_str())
    }

    pub fn package_full_name(&self) -> Option<&str> {
        match &self.kind {
            PeerKind::ExternalPackage { full_name, .. } => Some(full_name),
            PeerKind::LegacyProfile => None,
            #[cfg(target_os = "linux")]
            PeerKind::LinuxOwnedProxy { .. } => None,
        }
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub const fn port(&self) -> u16 {
        self.port
    }
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }
}

#[cfg(test)]
#[path = "sandbox_network_package_tests.rs"]
mod tests;
