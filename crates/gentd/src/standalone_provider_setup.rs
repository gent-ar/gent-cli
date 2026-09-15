use std::path::{Path, PathBuf};

use gent_protocol::DependencyProvider;

pub(crate) fn provider_prefix(data_dir: &Path) -> PathBuf {
    data_dir.join("providers").join("npm-global")
}

pub(crate) fn provider_executable(prefix: &Path, provider: DependencyProvider) -> Option<PathBuf> {
    let relative =
        gent_adapters::provider_platform::host_platform()?.executable(provider.as_str())?;
    Some(prefix.join(PACKAGE_DIRECTORY).join(relative))
}

#[cfg(windows)]
const PACKAGE_DIRECTORY: &str = "node_modules";
#[cfg(not(windows))]
const PACKAGE_DIRECTORY: &str = "lib/node_modules";

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gent_protocol::DependencyProvider;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn claude_runs_the_native_binary_from_its_platform_package() {
        let prefix = Path::new("/gent/providers/npm-global");
        assert_eq!(
            super::provider_executable(prefix, DependencyProvider::Claude).unwrap(),
            prefix.join("lib/node_modules/@anthropic-ai/claude-code-darwin-arm64/claude")
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn codex_runs_the_vendored_native_binary_from_its_platform_tarball() {
        let prefix = Path::new("/gent/providers/npm-global");
        assert_eq!(
            super::provider_executable(prefix, DependencyProvider::Codex).unwrap(),
            prefix.join("lib/node_modules/@openai/codex/vendor/aarch64-apple-darwin/bin/codex")
        );
    }
}
