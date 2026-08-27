use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::node_runtime_lock::AppNodeRuntimeLock;

use crate::daemon_bootstrap::ProviderLogin;

pub(crate) fn login(data_dir: &Path, provider: ProviderLogin) -> Result<(), String> {
    let provider = match provider {
        ProviderLogin::Claude => gent_types::AgentChatProvider::Claude,
        ProviderLogin::Codex => gent_types::AgentChatProvider::Codex,
    };
    let executable = ensure_provider(data_dir, provider)?;
    let mut command = Command::new(executable);
    if matches!(provider, gent_types::AgentChatProvider::Codex) {
        command.arg("--login");
    }
    command
        .status()
        .map_err(|error| format!("Gent could not start provider login: {error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "Provider login did not complete successfully".into())
}

pub(crate) fn ensure_provider(
    data_dir: &Path,
    provider: gent_types::AgentChatProvider,
) -> Result<PathBuf, String> {
    let runtime = AppNodeRuntimeLock::from_standalone_environment(data_dir)
        .map_err(|error| error.to_string())?;
    let npm = runtime
        .rechecked_npm_prefix()
        .map_err(|error| error.to_string())?;
    let prefix = npm.prefix().to_path_buf();
    let (executable_name, package) = match provider {
        gent_types::AgentChatProvider::Claude => ("claude", "@anthropic-ai/claude-code"),
        gent_types::AgentChatProvider::Codex => ("codex", "@openai/codex"),
        gent_types::AgentChatProvider::Claurst => {
            return Err("Claurst is not installed through the Node provider runtime".into());
        }
    };
    let executable = executable(&prefix, executable_name);
    if !executable.is_file() {
        let invocation = npm.install_package(package);
        let mut command = Command::new(invocation.executable);
        command
            .args(invocation.arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        npm.configure_command(&mut command)
            .map_err(|error| error.to_string())?;
        command
            .status()
            .map_err(|_| format!("Gent could not install {package}"))?
            .success()
            .then_some(())
            .ok_or_else(|| format!("Gent could not install {package}"))?;
    }
    executable
        .is_file()
        .then_some(executable)
        .ok_or_else(|| "Gent provider installation did not produce its expected executable".into())
}

pub(crate) fn installed_provider_executable(
    data_dir: &Path,
    provider: gent_types::AgentChatProvider,
) -> Option<PathBuf> {
    let executable_name = match provider {
        gent_types::AgentChatProvider::Claude => "claude",
        gent_types::AgentChatProvider::Codex => "codex",
        gent_types::AgentChatProvider::Claurst => return None,
    };
    let executable = executable(
        &data_dir.join("providers").join("npm-global"),
        executable_name,
    );
    executable.is_file().then_some(executable)
}

fn executable(prefix: &Path, provider: &str) -> PathBuf {
    prefix
        .join("bin")
        .join(format!("{provider}{}", executable_extension()))
}

#[cfg(windows)]
const fn executable_extension() -> &'static str {
    ".cmd"
}

#[cfg(not(windows))]
const fn executable_extension() -> &'static str {
    ""
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn provider_executables_stay_inside_the_gent_prefix() {
        let prefix = Path::new("/gent/providers/npm-global");
        assert_eq!(
            super::executable(prefix, "claude"),
            prefix
                .join("bin")
                .join(format!("claude{}", super::executable_extension()))
        );
        assert_eq!(
            super::executable(prefix, "codex"),
            prefix
                .join("bin")
                .join(format!("codex{}", super::executable_extension()))
        );
    }
}
