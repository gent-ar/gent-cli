use gent_types::{SandboxLaunchProfile, SandboxNetworkPolicy};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinuxBubblewrapMountPlan {
    arguments: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LinuxMountPlanError {
    #[error("Linux mount plan requires disabled networking and no gateway")]
    UnsupportedNetwork,
    #[error("Linux mount path is not a bounded normalized absolute path")]
    InvalidPath,
    #[error("Linux mount path overlaps a namespace-owned filesystem")]
    ReservedPath,
    #[error("Linux writable mount has no corresponding readable authority")]
    InvalidWritableMount,
}

impl LinuxBubblewrapMountPlan {
    pub fn from_profile(profile: &SandboxLaunchProfile) -> Result<Self, LinuxMountPlanError> {
        if !matches!(profile.network(), SandboxNetworkPolicy::Disabled)
            || profile.gateway().is_some()
        {
            return Err(LinuxMountPlanError::UnsupportedNetwork);
        }
        let workspace = text(profile.workspace_root())?;
        let mut readable = profile
            .readable_roots()
            .iter()
            .map(|p| text(p))
            .collect::<Result<Vec<_>, _>>()?;
        let mut writable = profile
            .writable_roots()
            .iter()
            .map(|p| text(p))
            .collect::<Result<Vec<_>, _>>()?;
        for resource in profile.resources() {
            let path = text(resource.root())?;
            readable.push(path.clone());
            if resource.writable() {
                writable.push(path);
            }
        }
        Self::compile(workspace, readable, writable)
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    fn compile(
        workspace: String,
        mut readable: Vec<String>,
        mut writable: Vec<String>,
    ) -> Result<Self, LinuxMountPlanError> {
        for path in std::iter::once(&workspace)
            .chain(&readable)
            .chain(&writable)
        {
            validate_path(path)?;
        }
        if !readable.contains(&workspace)
            || writable
                .iter()
                .any(|path| !readable.iter().any(|read| contains(read, path)))
        {
            return Err(LinuxMountPlanError::InvalidWritableMount);
        }
        readable.sort();
        readable.dedup();
        writable.sort();
        writable.dedup();
        let mut arguments = [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-net",
            "--unshare-uts",
            "--unshare-cgroup",
            "--disable-userns",
            "--cap-drop",
            "ALL",
            "--new-session",
            "--die-with-parent",
            "--clearenv",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
        ]
        .map(str::to_owned)
        .to_vec();
        for path in &readable {
            arguments.extend(["--ro-bind".into(), path.clone(), path.clone()]);
        }
        for path in &writable {
            arguments.extend(["--bind".into(), path.clone(), path.clone()]);
        }
        arguments.extend(["--chdir".into(), workspace]);
        Ok(Self { arguments })
    }
}

fn text(path: &std::path::Path) -> Result<String, LinuxMountPlanError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(LinuxMountPlanError::InvalidPath)
}

fn contains(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_path(path: &str) -> Result<(), LinuxMountPlanError> {
    if path.len() > 4096
        || !path.starts_with('/')
        || path == "/"
        || path.ends_with('/')
        || path.contains('\0')
        || path
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(LinuxMountPlanError::InvalidPath);
    }
    if ["/proc", "/dev", "/sys"]
        .iter()
        .any(|reserved| contains(reserved, path) || contains(path, reserved))
    {
        return Err(LinuxMountPlanError::ReservedPath);
    }
    Ok(())
}

#[cfg(test)]
#[path = "linux_sandbox_config_tests.rs"]
mod tests;
