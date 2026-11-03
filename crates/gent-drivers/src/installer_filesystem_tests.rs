use super::*;

#[cfg(windows)]
#[test]
fn all_npm_filesystem_arguments_are_literal_normal_paths() {
    let root = tempfile::tempdir().unwrap();
    let lock = runtime(root.path());
    let canonical_node = lock.node_path().to_string_lossy().into_owned();
    let npm = super::super::NpmGlobalPrefix::new(
        lock,
        PathBuf::from(r"\\?\C:\Gent\space #% 日本\npm-global"),
    );
    let package = ApprovedPackageInstall {
        provider: "codex".into(),
        package_name: "@openai/codex".into(),
        version: "1.0.0".into(),
        integrity: "sha512-test".into(),
        package_policy_digest_sha256: "a".repeat(64),
    };
    let packed = npm.pack(&package, Path::new(r"\\?\C:\Gent\space #% 日本\staging"));
    assert_eq!(packed.executable, canonical_node);
    assert_eq!(
        packed.arguments[0],
        super::super::npm_archive_path(&npm_cli(root.path()))
    );
    assert_eq!(packed.arguments[5], r"C:\Gent\space #% 日本\staging");
    assert_eq!(
        npm.install_archive(Path::new(r"C:\a.tgz")).arguments[5],
        r"C:\Gent\space #% 日本\npm-global"
    );
    assert_eq!(
        npm.install_package("@openai/codex").arguments[4],
        r"C:\Gent\space #% 日本\npm-global"
    );
    let unc = super::super::NpmGlobalPrefix::new(
        runtime(root.path()),
        PathBuf::from(r"\\?\UNC\server\share\npm-global"),
    );
    assert_eq!(
        unc.install_archive(Path::new(r"C:\a.tgz")).arguments[5],
        r"\\server\share\npm-global"
    );
    assert_eq!(
        unc.validate_prefix(),
        Err(super::super::InstallerError::UnsupportedPrefix)
    );
}

#[test]
fn private_npm_environment_paths_are_checked_and_all_host_controls_removed() {
    let root = tempfile::tempdir().unwrap();
    let prefix = root.path().join("space #% 日本");
    fs::create_dir(&prefix).unwrap();
    let prefix = prefix.canonicalize().unwrap();
    let lock = runtime(root.path());
    let node_bin = lock.node_path().parent().unwrap().to_owned();
    let npm = super::super::NpmGlobalPrefix::new(lock, prefix.canonicalize().unwrap());
    let mut command = Command::new("not-executed");
    command.env("NPM_CONFIG_LOGS_DIR", "host-controlled");
    command.env("npm_config_script_shell", "host-controlled");
    npm.configure_command(&mut command).unwrap();
    let env = command.get_envs().collect::<Vec<_>>();
    for (key, expected) in [
        ("npm_config_prefix", prefix.clone()),
        ("npm_config_cache", prefix.join(".npm-cache")),
        ("npm_config_userconfig", prefix.join(".npmrc")),
        ("npm_config_globalconfig", prefix.join("etc").join("npmrc")),
    ] {
        let value = env
            .iter()
            .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case(key))
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            value,
            std::ffi::OsStr::new(&super::super::npm_archive_path(&expected))
        );
    }
    for key in ["NPM_CONFIG_LOGS_DIR", "npm_config_script_shell"] {
        assert!(env.iter().any(
            |(name, value)| name.to_string_lossy().eq_ignore_ascii_case(key) && value.is_none()
        ));
    }
    let path = env
        .iter()
        .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATH"))
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        std::env::split_paths(path).next().unwrap(),
        PathBuf::from(super::super::npm_archive_path(&node_bin))
    );
    assert!(!prefix.join(".npmrc").exists());
    assert!(!prefix.join("etc").exists());
}
