pub struct LaunchProfile {
    pub executable: String,
    pub arguments: Vec<String>,
    pub cwd: String,
    pub readwrite: Vec<String>,
    pub readonly: Vec<String>,
    pub denied: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub timeout_ms: u32,
}

fn absolute(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\0')
        && !path.split('/').any(|part| part == ".." || part == ".")
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
impl LaunchProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !absolute(&self.executable) || !absolute(&self.cwd) || self.timeout_ms == 0 {
            return Err("absolute executable/cwd and finite nonzero timeout required".into());
        }
        if self.arguments.iter().any(|arg| arg.contains('\0')) {
            return Err("NUL argument".into());
        }
        for path in self
            .readwrite
            .iter()
            .chain(&self.readonly)
            .chain(&self.denied)
        {
            if !absolute(path) || path == "/" {
                return Err("explicit non-root absolute paths required".into());
            }
        }
        if self
            .environment
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err("invalid environment entry".into());
        }
        let home = self
            .environment
            .iter()
            .filter(|(key, _)| key == "HOME")
            .collect::<Vec<_>>();
        if home.len() != 1
            || !absolute(&home[0].1)
            || !self.readwrite.iter().any(|root| {
                home[0].1 == *root
                    || home[0]
                        .1
                        .starts_with(&(root.trim_end_matches('/').to_string() + "/"))
            })
        {
            return Err("one private HOME under an explicit writable root required".into());
        }
        Ok(())
    }
    pub fn command(&self) -> String {
        std::iter::once("exec".to_string())
            .chain(std::iter::once(quote(&self.executable)))
            .chain(self.arguments.iter().map(|arg| quote(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

use mxc_sdk::v1::{
    self, ContainerRequest, Containment, FilesystemPolicy, MxcProcess, NetworkAction,
    NetworkEgressPolicy, NetworkIngressPolicy, NetworkPolicy, SpawnOptions, TelemetryConfig,
};

/// Explicit platform selection; never select Process or downgrade containment.
fn containment() -> Result<Containment, String> {
    #[cfg(target_os = "linux")]
    {
        Ok(Containment::Bubblewrap)
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(Containment::Seatbelt(Default::default()));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err("MXC Linux adapter supports Linux/macOS desktop only".into())
    }
}

/// Deny-network request primitive. The owned launcher supplies reviewed proxy
/// enforcement and private per-run credential injection before provider release.
pub fn request(profile: LaunchProfile) -> Result<ContainerRequest, String> {
    profile.validate()?;
    let mut request = ContainerRequest::new(profile.command());
    request.containment = containment()?;
    request.filesystem = Some(FilesystemPolicy {
        readwrite_paths: profile.readwrite,
        readonly_paths: profile.readonly,
        denied_paths: profile.denied,
        clear_policy_on_exit: Some(true),
    });
    request.network = Some(NetworkPolicy {
        egress: Some(NetworkEgressPolicy {
            default: Some(NetworkAction::Deny),
            allow: None,
            deny: None,
        }),
        ingress: Some(NetworkIngressPolicy {
            default: Some(NetworkAction::Deny),
            host_loopback: Some(NetworkAction::Deny),
        }),
        runtime_config: None,
    });
    request.timeout_ms = Some(profile.timeout_ms);
    request.working_directory = Some(profile.cwd);
    request.environment = Some(profile.environment);
    request.inherit_default_environment = Some(false);
    Ok(request)
}

/// Returns the native process/stream owner. Caller must drain both output streams
/// concurrently, supervise timeout/cancellation and retain all policy/lease pins.
/// A PID alone is deliberately not represented as Gent `SandboxEnforcement` proof.
pub fn spawn(profile: LaunchProfile) -> Result<MxcProcess, String> {
    let request = request(profile)?;
    v1::spawn(
        request,
        SpawnOptions {
            telemetry: Some(TelemetryConfig {
                enabled: Some(false),
            }),
        },
    )
    .map_err(|error| error.to_string())
}

/// Candidate local gateway path. This changes no Gent policy/admission gate.
/// The engine must enforce proxy-only egress; a missing dependency is an error.
/// Caller retains the actual gateway lease and injects only per-run credentials.
pub fn request_with_gateway(
    profile: LaunchProfile,
    gateway: std::net::SocketAddrV4,
) -> Result<ContainerRequest, String> {
    if !gateway.ip().is_loopback() || gateway.port() == 0 {
        return Err("finite IPv4 loopback gateway required".into());
    }
    let mut request = request(profile)?;
    request
        .network
        .as_mut()
        .expect("deny policy constructed above")
        .runtime_config = Some(v1::NetworkRuntimeConfig {
        network_proxy: Some(format!("http://{gateway}")),
    });
    Ok(request)
}

impl std::fmt::Debug for LaunchProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchProfile")
            .field("arguments", &self.arguments.len())
            .field("environment", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> LaunchProfile {
        LaunchProfile {
            executable: "/usr/bin/python3".into(),
            arguments: vec!["a'b;$(touch /tmp/escape)".into()],
            cwd: "/tmp/owned".into(),
            readwrite: vec!["/tmp/owned".into()],
            readonly: vec![],
            denied: vec![],
            environment: vec![("HOME".into(), "/tmp/owned/home".into())],
            timeout_ms: 1000,
        }
    }
    #[test]
    fn valid_private_profile() {
        assert!(profile().validate().is_ok());
    }
    #[test]
    fn shell_atoms_are_quoted() {
        assert_eq!(
            profile().command(),
            "exec '/usr/bin/python3' 'a'\\''b;$(touch /tmp/escape)'"
        );
    }
    #[test]
    fn broad_or_relative_paths_rejected() {
        for path in ["/", "relative", "/tmp/../escape", "/tmp/./escape"] {
            let mut p = profile();
            p.readwrite.push(path.into());
            assert!(p.validate().is_err());
        }
    }
    #[test]
    fn home_and_nul_rejected() {
        let mut p = profile();
        p.environment[0].1 = "/tmp/unowned/home".into();
        assert!(p.validate().is_err());
        let mut p = profile();
        p.arguments.push("x\0y".into());
        assert!(p.validate().is_err());
    }
}
