use std::path::PathBuf;

pub const CODEX_PACKAGE: &str = "@openai/codex";
const CLAUDE_PACKAGE_PREFIX: &str = "@anthropic-ai/claude-code-";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderPlatform {
    pub npm: &'static str,
    pub codex_triple: &'static str,
    pub windows: bool,
}

pub const PLATFORMS: [ProviderPlatform; 6] = [
    ProviderPlatform {
        npm: "darwin-arm64",
        codex_triple: "aarch64-apple-darwin",
        windows: false,
    },
    ProviderPlatform {
        npm: "darwin-x64",
        codex_triple: "x86_64-apple-darwin",
        windows: false,
    },
    ProviderPlatform {
        npm: "linux-arm64",
        codex_triple: "aarch64-unknown-linux-musl",
        windows: false,
    },
    ProviderPlatform {
        npm: "linux-x64",
        codex_triple: "x86_64-unknown-linux-musl",
        windows: false,
    },
    ProviderPlatform {
        npm: "win32-arm64",
        codex_triple: "aarch64-pc-windows-msvc",
        windows: true,
    },
    ProviderPlatform {
        npm: "win32-x64",
        codex_triple: "x86_64-pc-windows-msvc",
        windows: true,
    },
];

#[must_use]
pub const fn host_platform() -> Option<ProviderPlatform> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(PLATFORMS[0])
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(PLATFORMS[1])
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "aarch64"
    )) {
        Some(PLATFORMS[2])
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        Some(PLATFORMS[3])
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        Some(PLATFORMS[4])
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some(PLATFORMS[5])
    } else {
        None
    }
}

impl ProviderPlatform {
    #[must_use]
    pub fn package_name(self, provider: &str) -> Option<String> {
        match provider {
            "claude" => Some(format!("{CLAUDE_PACKAGE_PREFIX}{}", self.npm)),
            "codex" => Some(CODEX_PACKAGE.into()),
            _ => None,
        }
    }

    #[must_use]
    pub fn accepts(self, provider: &str, package_name: &str, version: &str) -> bool {
        match provider {
            "claude" => self.package_name(provider).as_deref() == Some(package_name),
            "codex" => {
                package_name == CODEX_PACKAGE
                    && version
                        .strip_suffix(self.npm)
                        .is_some_and(|core| core.ends_with('-') && core.len() > 1)
            }
            _ => false,
        }
    }

    #[must_use]
    pub fn executable(self, provider: &str) -> Option<PathBuf> {
        let extension = if self.windows { ".exe" } else { "" };
        match provider {
            "claude" => Some(
                PathBuf::from(format!("{CLAUDE_PACKAGE_PREFIX}{}", self.npm))
                    .join(format!("claude{extension}")),
            ),
            "codex" => Some(
                PathBuf::from(CODEX_PACKAGE)
                    .join("vendor")
                    .join(self.codex_triple)
                    .join("bin")
                    .join(format!("codex{extension}")),
            ),
            _ => None,
        }
    }
}

#[must_use]
pub fn known_package(provider: &str, package_name: &str, version: &str) -> bool {
    PLATFORMS
        .iter()
        .any(|platform| platform.accepts(provider, package_name, version))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use super::host_platform;
    use super::{PLATFORMS, known_package};

    #[test]
    fn claude_is_bound_to_its_platform_package_and_native_binary() {
        let darwin = PLATFORMS[0];
        assert!(darwin.accepts(
            "claude",
            "@anthropic-ai/claude-code-darwin-arm64",
            "2.1.270"
        ));
        assert!(!darwin.accepts("claude", "@anthropic-ai/claude-code", "2.1.270"));
        assert!(!darwin.accepts("claude", "@anthropic-ai/claude-code-linux-x64", "2.1.270"));
        assert_eq!(
            darwin.executable("claude").unwrap(),
            PathBuf::from("@anthropic-ai/claude-code-darwin-arm64/claude")
        );
        assert_eq!(
            PLATFORMS[5].executable("claude").unwrap(),
            PathBuf::from("@anthropic-ai/claude-code-win32-x64/claude.exe")
        );
    }

    #[test]
    fn codex_is_bound_to_the_platform_tarball_version_and_vendored_native_binary() {
        let darwin = PLATFORMS[0];
        assert!(darwin.accepts("codex", "@openai/codex", "0.153.4-darwin-arm64"));
        assert!(!darwin.accepts("codex", "@openai/codex", "0.153.4"));
        assert!(!darwin.accepts("codex", "@openai/codex", "0.153.4-darwin-x64"));
        assert!(!darwin.accepts("codex", "@openai/codex", "darwin-arm64"));
        assert!(!darwin.accepts(
            "codex",
            "@openai/codex-darwin-arm64",
            "0.153.4-darwin-arm64"
        ));
        assert_eq!(
            darwin.executable("codex").unwrap(),
            PathBuf::from("@openai/codex/vendor/aarch64-apple-darwin/bin/codex")
        );
        assert_eq!(
            PLATFORMS[3].executable("codex").unwrap(),
            PathBuf::from("@openai/codex/vendor/x86_64-unknown-linux-musl/bin/codex")
        );
        assert!(known_package(
            "codex",
            "@openai/codex",
            "0.153.4-win32-arm64"
        ));
        assert!(!known_package("claurst", "claurst", "0.1.7"));
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn this_host_selects_darwin_arm64() {
        assert_eq!(host_platform().unwrap().npm, "darwin-arm64");
    }
}
