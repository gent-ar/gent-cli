use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use gent_ports::ApprovedPackageInstall;

#[cfg(windows)]
#[test]
fn verified_archive_argument_uses_npm_compatible_drive_and_unc_paths() {
    let root = tempfile::tempdir().unwrap();
    let npm = super::NpmGlobalPrefix::new(runtime(root.path()), root.path().join("prefix"));
    for (archive, expected) in [
        (
            r"\\?\X:\Dev\a folder\提供者.tgz",
            r"X:\Dev\a%20folder\%E6%8F%90%E4%BE%9B%E8%80%85.tgz",
        ),
        (
            r"\\?\UNC\server\share\a folder\provider.tgz",
            "file:////server/share/a%20folder/provider.tgz",
        ),
        (r"X:\Dev\provider.tgz", r"X:\Dev\provider.tgz"),
        (
            r"\\server\share\provider.tgz",
            "file:////server/share/provider.tgz",
        ),
        (
            r"\\?\X:\a#b\100%\provider.tgz",
            r"X:\a%23b\100%25\provider.tgz",
        ),
        (
            r"\\?\UNC\server\share\space #% 日本.tgz",
            "file:////server/share/space%20%23%25%20%E6%97%A5%E6%9C%AC.tgz",
        ),
        (r"relative\provider.tgz", r"relative\provider.tgz"),
        (
            r"\\?\Volume{example}\provider.tgz",
            r"\\?\Volume{example}\provider.tgz",
        ),
    ] {
        let invocation = npm.install_archive(Path::new(archive));
        assert_eq!(invocation.arguments[7], expected);
        assert_eq!(invocation.arguments[6], "--offline");
        assert_eq!(invocation.arguments[2], "--ignore-scripts");
        assert_eq!(invocation.arguments.len(), 8);
    }
}

#[test]
fn npm_commands_use_only_the_private_prefix_and_fixed_arguments() {
    let root = tempfile::tempdir().unwrap();
    let npm = super::NpmGlobalPrefix::new(
        runtime(root.path()),
        PathBuf::from("/private/gentd/providers/npm-global"),
    );
    let package = ApprovedPackageInstall {
        provider: "codex".into(),
        package_name: "@openai/codex".into(),
        version: "0.147.0".into(),
        integrity: "sha512-test".into(),
        package_policy_digest_sha256: "a".repeat(64),
    };
    let packed = npm.pack(&package, std::path::Path::new("/private/staging"));
    assert!(Path::new(&packed.executable).ends_with("bin/node"));
    assert_eq!(
        packed.arguments[0],
        super::npm_archive_path(&npm_cli(root.path()))
    );
    assert_eq!(packed.arguments[1], "pack");
    assert_eq!(packed.arguments[2], "--ignore-scripts");
    assert_eq!(packed.arguments[3], "--json");
    assert_eq!(packed.arguments[4], "--pack-destination");
    assert_eq!(packed.arguments[5], "/private/staging");
    assert_eq!(packed.arguments[6], "@openai/codex@0.147.0");
    let installed = npm.install_archive(std::path::Path::new("/private/staging/codex.tgz"));
    assert_eq!(
        installed.arguments[5],
        "/private/gentd/providers/npm-global"
    );
    assert_eq!(installed.arguments[6], "--offline");
    assert_eq!(installed.arguments[7], "/private/staging/codex.tgz");
    assert_eq!(installed.arguments[2], "--ignore-scripts");
    let package = npm.install_package("@openai/codex");
    assert_eq!(package.arguments[1], "install");
    assert_eq!(package.arguments[2], "--global");
    assert_eq!(package.arguments[3], "--prefix");
    assert_eq!(package.arguments[4], "/private/gentd/providers/npm-global");
    assert_eq!(package.arguments[7], "@openai/codex");
}

#[test]
fn private_npm_command_clears_host_node_and_npm_controls() {
    let root = tempfile::tempdir().unwrap();
    let prefix = root.path().join("private-prefix");
    let npm = super::NpmGlobalPrefix::new(runtime(root.path()), prefix.clone());
    let mut command = Command::new("ignored");
    for name in ["NODE_OPTIONS", "NODE_PATH", "NPM_CONFIG_REGISTRY"] {
        command.env(name, "host-value");
    }
    npm.configure_command(&mut command).unwrap();
    let values = command
        .get_envs()
        .map(|(name, value)| (name.to_string_lossy().into_owned(), value.is_some()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(values.get("NODE_OPTIONS"), Some(&false));
    assert_eq!(values.get("NODE_PATH"), Some(&false));
    assert_eq!(values.get("NPM_CONFIG_REGISTRY"), Some(&false));
    assert_eq!(
        values.get("npm_config_prefix"),
        Some(&true),
        "npm receives only the private prefix"
    );
    assert_eq!(values.get("npm_config_userconfig"), Some(&true));
}

#[cfg(unix)]
#[test]
fn private_npm_path_contains_only_locked_node_and_system_interpreters() {
    let root = tempfile::tempdir().unwrap();
    let npm = super::NpmGlobalPrefix::new(runtime(root.path()), root.path().join("prefix"));
    let mut command = Command::new("ignored");
    npm.configure_command(&mut command).unwrap();
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "PATH").then_some(value))
        .flatten()
        .unwrap();
    assert_eq!(
        std::env::split_paths(path).collect::<Vec<_>>(),
        vec![
            root.path().join("bin").canonicalize().unwrap(),
            "/usr/bin".into(),
            "/bin".into()
        ]
    );
}

fn runtime(root: &Path) -> crate::node_runtime_lock::NodeRuntimeLock {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("node"), "node").unwrap();
    fs::write(bin.join(npm_name()), "npm").unwrap();
    let cli = root.join("lib/node_modules/npm/bin");
    fs::create_dir_all(&cli).unwrap();
    fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
    crate::node_runtime_lock::NodeRuntimeLock::capture(&bin.join("node")).unwrap()
}

fn npm_cli(root: &Path) -> PathBuf {
    root.join("lib/node_modules/npm/bin/npm-cli.js")
        .canonicalize()
        .unwrap()
}

#[cfg(windows)]
const fn npm_name() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
const fn npm_name() -> &'static str {
    "npm"
}

#[path = "installer_filesystem_tests.rs"]
mod filesystem_tests;
