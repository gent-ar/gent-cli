use std::process::Command;

use crate::supervisor::SupervisorError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NodeSearch {
    Inherited,
    Locked(std::path::PathBuf),
    First(std::path::PathBuf),
}

impl NodeSearch {
    pub(super) fn configure(&self, command: &mut Command) -> Result<(), SupervisorError> {
        match self {
            Self::Inherited => Ok(()),
            Self::Locked(node_bin) => configure_locked_node_environment(command, node_bin),
            Self::First(node_bin) => {
                command.env("PATH", node_first_path(node_bin)?);
                Ok(())
            }
        }
    }
}

pub fn configure_locked_node_environment(
    command: &mut Command,
    node_bin: &std::path::Path,
) -> Result<(), SupervisorError> {
    let path = locked_node_path(node_bin)?;
    command.env("PATH", path);
    for variable in [
        "NODE_OPTIONS",
        "NODE_PATH",
        "npm_config_prefix",
        "NPM_CONFIG_PREFIX",
        "npm_config_userconfig",
        "NPM_CONFIG_USERCONFIG",
        "npm_config_globalconfig",
        "NPM_CONFIG_GLOBALCONFIG",
        "npm_config_registry",
        "NPM_CONFIG_REGISTRY",
        "npm_config_proxy",
        "NPM_CONFIG_PROXY",
        "npm_config_https_proxy",
        "NPM_CONFIG_HTTPS_PROXY",
    ] {
        command.env_remove(variable);
    }
    Ok(())
}

pub(super) const PACKAGE_MANAGER_PROVENANCE: [&str; 5] = [
    "CODEX_MANAGED_BY_NPM",
    "CODEX_MANAGED_BY_BUN",
    "CODEX_MANAGED_BY_PNPM",
    "CODEX_MANAGED_BY_VITE_PLUS",
    "CODEX_MANAGED_PACKAGE_ROOT",
];

pub(super) fn provider_environment(command: &mut Command, provider: &str) {
    for variable in PACKAGE_MANAGER_PROVENANCE {
        command.env_remove(variable);
    }
    command.envs(
        crate::launch_spec::provider_environment(provider)
            .iter()
            .copied(),
    );
}

fn node_first_path(node_bin: &std::path::Path) -> Result<std::ffi::OsString, SupervisorError> {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(
        std::iter::once(node_bin.to_path_buf()).chain(std::env::split_paths(&inherited)),
    )
    .map_err(|_| SupervisorError::Launch("Node directory cannot form PATH".into()))
}

fn locked_node_path(node_bin: &std::path::Path) -> Result<std::ffi::OsString, SupervisorError> {
    #[cfg(unix)]
    let paths = [node_bin.to_path_buf(), "/usr/bin".into(), "/bin".into()];
    #[cfg(windows)]
    let paths = [node_bin.to_path_buf()];
    std::env::join_paths(paths)
        .map_err(|_| SupervisorError::Launch("locked Node path cannot form PATH".into()))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, process::Command};

    use super::{PACKAGE_MANAGER_PROVENANCE, provider_environment};

    #[test]
    fn provider_launches_never_inherit_codex_package_manager_provenance() {
        let mut command = Command::new("codex");
        for variable in PACKAGE_MANAGER_PROVENANCE {
            command.env(variable, "/somewhere/else");
        }
        command.env("HOME", "/home/user");
        provider_environment(&mut command, "codex");
        let environment: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        for variable in PACKAGE_MANAGER_PROVENANCE {
            assert!(
                environment.contains(&(OsStr::new(variable), None)),
                "{variable}"
            );
        }
        assert!(environment.contains(&(OsStr::new("HOME"), Some(OsStr::new("/home/user")))));
    }

    #[test]
    fn claude_launches_report_their_session_state() {
        let mut command = Command::new("claude");
        provider_environment(&mut command, "claude");
        assert!(command.get_envs().any(|entry| entry
            == (
                OsStr::new("CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS"),
                Some(OsStr::new("1"))
            )));
    }
}
