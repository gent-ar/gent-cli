use std::path::{Path, PathBuf};

use gent_protocol::DependencyProvider;

pub(crate) fn provider_prefix(data_dir: &Path) -> PathBuf {
    data_dir.join("providers").join("npm-global")
}

/// Derives one private attempt from the full accepted command identity, never a client path.
pub(crate) fn provider_generation_prefix(
    legacy_prefix: &Path,
    fingerprint: &str,
) -> Option<PathBuf> {
    crate::private_provider_lock_validation::valid_digest(fingerprint).then_some(())?;
    Some(
        legacy_prefix
            .parent()?
            .join("generations")
            .join(fingerprint)
            .join("npm-global"),
    )
}

/// Reads only the exact receipt-bound generation; no discovery or directory scanning.
pub(crate) fn verified_generation_prefix(
    legacy_prefix: &Path,
    fingerprint: &str,
) -> Option<PathBuf> {
    provider_generation_prefix(legacy_prefix, fingerprint)?;
    let providers = legacy_prefix.parent()?;
    let mut spelled = providers.to_path_buf();
    let mut canonical = plain_directory(providers)?;
    for part in ["generations", fingerprint, "npm-global"] {
        spelled.push(part);
        canonical = canonical.join(part);
        if plain_directory(&spelled)? != canonical {
            return None;
        }
    }
    Some(canonical)
}

pub(crate) fn verified_legacy_prefix(prefix: &Path) -> Option<PathBuf> {
    let expected = plain_directory(prefix.parent()?)?.join("npm-global");
    (plain_directory(prefix)? == expected).then_some(expected)
}

fn plain_directory(path: &Path) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_dir()).then_some(())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        (metadata.file_attributes() & 0x400 == 0).then_some(())?;
    }
    path.canonicalize().ok()
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
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use std::path::Path;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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
