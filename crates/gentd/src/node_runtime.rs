//! App-supplied Node discovery for Gent-owned public-provider provisioning.

use std::{env, ffi::OsString, fs, path::Path};

use gent_drivers::installer::NpmGlobalPrefix;

const NODE_BINARY_ENV: &str = "GENT_NODE_BINARY";

/// Resolves a supplied Node binary to its sibling `npm` and a private Gent prefix.
pub(crate) fn private_npm_prefix(data_dir: &Path) -> Option<NpmGlobalPrefix> {
    private_npm_prefix_for(env::var_os(NODE_BINARY_ENV), data_dir)
}

fn private_npm_prefix_for(node: Option<OsString>, data_dir: &Path) -> Option<NpmGlobalPrefix> {
    let node = fs::canonicalize(node?).ok()?;
    let npm = fs::canonicalize(node.parent()?.join(npm_name())).ok()?;
    npm.is_file()
        .then(|| NpmGlobalPrefix::new(npm, data_dir.join("providers").join("npm-global")))
}

#[cfg(windows)]
const fn npm_name() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
const fn npm_name() -> &'static str {
    "npm"
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gent_ports::ApprovedPackageInstall;

    use super::private_npm_prefix_for;

    #[test]
    #[cfg(unix)]
    fn supplied_node_uses_its_sibling_npm_and_a_private_prefix() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let node = bin.join("node");
        let npm = bin.join("npm");
        fs::write(&node, "node").unwrap();
        fs::write(&npm, "npm").unwrap();
        let runtime =
            private_npm_prefix_for(Some(node.into_os_string()), &root.path().join("gentd"))
                .unwrap();
        let package = ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
        };
        assert!(runtime.install(&package).executable.ends_with("/npm"));
        assert!(runtime.install(&package).arguments[3].ends_with("providers/npm-global"));
    }
}
