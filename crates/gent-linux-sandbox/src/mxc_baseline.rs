//! Explicit coverage of MXC1.0.1's fixed OS read baseline, never auto-granted.
use gent_types::{SandboxLaunchProfile, SandboxResourceGrant};
use std::path::PathBuf;
pub const PINNED_MXC_REV: &str = "fda29ac63bc306b7f97f3b86b4b63d44f7f40cf2";
pub const BASELINE: &[&str] = &[
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/lib32",
    "/usr/lib64",
    "/usr/libexec",
    "/usr/share",
    "/etc",
    "/run/systemd/resolve",
    "/run/NetworkManager",
    "/run/resolvconf",
    "/mnt/wsl/resolv.conf",
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineError {
    Uncovered(Vec<PathBuf>),
    InspectionFailed,
    WritableSystemGrant,
}
pub fn required_existing_roots() -> Result<Vec<PathBuf>, BaselineError> {
    let mut roots = Vec::new();
    for path in BASELINE {
        match std::fs::canonicalize(path) {
            Ok(path) => roots.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(BaselineError::InspectionFailed),
        }
    }
    roots.sort();
    roots.dedup();
    let mut minimal = Vec::new();
    for root in roots {
        if !minimal.iter().any(|p: &PathBuf| root.starts_with(p)) {
            minimal.push(root);
        }
    }
    Ok(minimal)
}
/// Only already-approved resource grants count. This function adds no authority.
pub fn validate(profile: &SandboxLaunchProfile) -> Result<Vec<PathBuf>, BaselineError> {
    let required = required_existing_roots()?;
    for path in profile.writable_roots() {
        let path = std::fs::canonicalize(path).map_err(|_| BaselineError::InspectionFailed)?;
        if required
            .iter()
            .any(|root| root.starts_with(&path) || path.starts_with(root))
        {
            return Err(BaselineError::WritableSystemGrant);
        }
    }
    let mut approved = Vec::new();
    for grant in profile.resources() {
        match grant {
            SandboxResourceGrant::RuntimeReadOnly(path)
            | SandboxResourceGrant::ConfigurationReadOnly(path) => approved
                .push(std::fs::canonicalize(path).map_err(|_| BaselineError::InspectionFailed)?),
            SandboxResourceGrant::PrivateStateReadWrite(path) => {
                let path =
                    std::fs::canonicalize(path).map_err(|_| BaselineError::InspectionFailed)?;
                if required
                    .iter()
                    .any(|p| p.starts_with(&path) || path.starts_with(p))
                {
                    return Err(BaselineError::WritableSystemGrant);
                }
            }
        }
    }
    let missing = required
        .iter()
        .filter(|root| !approved.iter().any(|p| root.starts_with(p)))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(BaselineError::Uncovered(missing));
    }
    Ok(required)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuSemantics {
    StrictTotal,
    MonitoredTotal { poll_ms: u16, wall_ms: u32 },
}
impl CpuSemantics {
    pub fn validate(self) -> Result<Self, &'static str> {
        match self {
            Self::StrictTotal => {
                Err("strict cumulative CPU ceiling is unsupported by this MXC backend")
            }
            Self::MonitoredTotal { poll_ms, wall_ms }
                if (1..=100).contains(&poll_ms) && (100..=600_000).contains(&wall_ms) =>
            {
                Ok(self)
            }
            Self::MonitoredTotal { .. } => Err("invalid monitored CPU policy"),
        }
    }
}
