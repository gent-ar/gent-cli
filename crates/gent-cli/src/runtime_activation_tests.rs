use super::*;
use std::ffi::OsString;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TARGET: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const TARGET: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const TARGET: &str = "x86_64-unknown-linux-gnu";

#[cfg(not(windows))]
#[test]
fn activation_selects_a_complete_newer_bootstrap_without_downgrading() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| name.to_owned());
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.21","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("runtime");
    let selected = activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    assert!(selected.exists());
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.20","target":"{TARGET}"}}"#),
    )
    .unwrap();
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    let expected = format!("v0.1.21-{TARGET}");
    assert_eq!(
        selected_release(&root).unwrap().as_deref(),
        Some(expected.as_str())
    );
}

#[cfg(not(windows))]
fn verify_scheduler(root: &Path, data_dir: &Path) -> Result<(), String> {
    if active_cli(root) == root.join("current/gent") && data_dir.is_dir() {
        Ok(())
    } else {
        Err("wrong active runtime".into())
    }
}

#[test]
fn activation_passes_data_directory_before_the_update_subcommand() {
    let data_dir = Path::new("/tmp/gent-data");
    assert_eq!(
        enable_update_arguments(data_dir),
        vec![
            OsString::from("--data-dir"),
            OsString::from("/tmp/gent-data"),
            OsString::from("update"),
            OsString::from("auto"),
            OsString::from("enable"),
        ]
    );
}

#[cfg(not(windows))]
#[test]
fn activation_keeps_the_runtime_available_when_scheduler_refresh_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| name.to_owned());
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.23","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("runtime");
    assert!(
        activate_at(&bootstrap, &root, temporary.path(), |_, _| Err(
            "scheduler unavailable".into()
        ))
        .is_ok()
    );
}

#[cfg(not(windows))]
#[test]
fn isolated_runtime_root_activation_never_takes_over_update_scheduling() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| name.to_owned());
    let marker = temporary.path().join("scheduler-invoked");
    fs::write(
        bootstrap.join("gent"),
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(bootstrap.join("gent"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.26","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("isolated-runtime");
    let daemon = activate(bootstrap, Some(root.clone()), temporary.path().join("data")).unwrap();
    assert_eq!(daemon, root.join("current/gentd"));
    assert!(!marker.exists());
}

#[cfg(not(windows))]
#[test]
fn activation_refreshes_the_runtime_root_auto_update_helper() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| format!("new-{name}"));
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.24","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("runtime");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(auto_update_helper_name()), "old-helper").unwrap();
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(auto_update_helper_name())).unwrap(),
        format!("new-{}", auto_update_helper_name())
    );
}

#[cfg(not(windows))]
#[test]
fn activation_reuses_a_signed_release_with_the_same_verified_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| name.to_owned());
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.25","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("runtime");
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    fs::write(
        bootstrap.join("gent"),
        "macOS code signature changes executable bytes",
    )
    .unwrap();
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    let expected = format!("v0.1.25-{TARGET}");
    assert_eq!(
        selected_release(&root).unwrap().as_deref(),
        Some(expected.as_str())
    );
}

#[cfg(not(windows))]
#[test]
fn activation_restages_a_rebuilt_release_that_kept_its_version() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    write_bootstrap(&bootstrap, |name| name.to_owned());
    fs::write(
        bootstrap.join("bootstrap.json"),
        format!(r#"{{"version":"v0.1.34","target":"{TARGET}"}}"#),
    )
    .unwrap();
    let root = temporary.path().join("runtime");
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    let release = root.join("releases").join(format!("v0.1.34-{TARGET}"));
    let witness = release.join("staged-once");
    fs::write(&witness, "witness").unwrap();

    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    assert!(witness.exists(), "identical bytes must be reused");

    fs::write(bootstrap.join("gentd"), "rebuilt gentd").unwrap();
    activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
    assert!(!witness.exists(), "rebuilt bytes must be restaged");
    assert_eq!(
        fs::read_to_string(release.join("gentd")).unwrap(),
        "rebuilt gentd"
    );
    assert_eq!(
        selected_release(&root).unwrap().as_deref(),
        Some(format!("v0.1.34-{TARGET}").as_str())
    );
    assert!(active_daemon(&root).exists());
}

#[cfg(not(windows))]
fn write_bootstrap(bootstrap: &Path, contents: fn(&str) -> String) {
    for name in required_files() {
        let path = bootstrap.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents(name)).unwrap();
    }
}

// Windows has no `root/current` symlink — `select_release` writes only
// `current.json` (a JSON pointer; real symlinks need elevated privilege
// there) plus copies of `gent-launcher.exe` into `root/bin`. A prior bug had
// `refresh_auto_update_helper` read the helper from `root/current/<name>`
// unconditionally, which doesn't exist on Windows: `fs::symlink_metadata`
// fails on the missing `current` path *component*, not a missing file, and
// that raw `ERROR_PATH_NOT_FOUND` reached the user as
// "gent: The system cannot find the path specified. (os error 3)" with no
// indication it came from the update-scheduler refresh at all — every other
// test in this file is `#[cfg(not(windows))]`, so nothing caught it. These
// mirror the ones above on Windows's own path scheme.
#[cfg(windows)]
mod windows {
    use super::*;

    fn write_bootstrap(bootstrap: &Path, contents: fn(&str) -> String) {
        for name in required_files() {
            let path = bootstrap.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents(name)).unwrap();
        }
    }

    fn verify_scheduler(root: &Path, data_dir: &Path) -> Result<(), String> {
        if active_cli(root) == root.join("bin/gent.exe") && data_dir.is_dir() {
            Ok(())
        } else {
            Err("wrong active runtime".into())
        }
    }

    #[test]
    fn activation_selects_a_complete_newer_bootstrap_without_downgrading() {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        write_bootstrap(&bootstrap, |name| name.to_owned());
        fs::write(
            bootstrap.join("bootstrap.json"),
            r#"{"version":"v0.1.21","target":"x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        let root = temporary.path().join("runtime");
        let selected = activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
        assert!(selected.exists());
        fs::write(
            bootstrap.join("bootstrap.json"),
            r#"{"version":"v0.1.20","target":"x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
        assert_eq!(
            selected_release(&root).unwrap().as_deref(),
            Some("v0.1.21-x86_64-pc-windows-msvc")
        );
    }

    /// The regression test for the bug this fix addresses: activation must
    /// not fail, and must refresh the helper file at `root`, when the only
    /// pointer to the active release is `current.json` (no `current` symlink).
    #[test]
    fn activation_refreshes_the_runtime_root_auto_update_helper() {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        write_bootstrap(&bootstrap, |name| format!("new-{name}"));
        fs::write(
            bootstrap.join("bootstrap.json"),
            r#"{"version":"v0.1.24","target":"x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        let root = temporary.path().join("runtime");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(auto_update_helper_name()), "old-helper").unwrap();
        activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(auto_update_helper_name())).unwrap(),
            format!("new-{}", auto_update_helper_name())
        );
    }

    #[test]
    fn activation_keeps_the_runtime_available_when_scheduler_refresh_fails() {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        write_bootstrap(&bootstrap, |name| name.to_owned());
        fs::write(
            bootstrap.join("bootstrap.json"),
            r#"{"version":"v0.1.23","target":"x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        let root = temporary.path().join("runtime");
        assert!(
            activate_at(&bootstrap, &root, temporary.path(), |_, _| Err(
                "scheduler unavailable".into()
            ))
            .is_ok()
        );
    }

    #[test]
    fn isolated_runtime_root_activation_never_takes_over_update_scheduling() {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        write_bootstrap(&bootstrap, |name| name.to_owned());
        let marker = temporary.path().join("scheduler-invoked");
        fs::write(bootstrap.join("gent.exe"), "not actually invoked").unwrap();
        fs::write(
            bootstrap.join("bootstrap.json"),
            r#"{"version":"v0.1.26","target":"x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        let root = temporary.path().join("isolated-runtime");
        let daemon = activate(bootstrap, Some(root.clone()), temporary.path().join("data")).unwrap();
        assert_eq!(daemon, root.join("bin/gentd.exe"));
        assert!(!marker.exists());
    }

    #[test]
    fn activation_refuses_a_bootstrap_without_its_packaged_runtime_or_authority() {
        for missing in [
            "runtime/node/bin/node.exe",
            "runtime/claurst/claurst.exe",
            "runtime/claurst/llama/llama-server.exe",
            "authority/ordinary-authority.json",
            "authority/root-keys.json",
        ] {
            let temporary = tempfile::tempdir().unwrap();
            let bootstrap = temporary.path().join("bootstrap");
            write_bootstrap(&bootstrap, |name| name.to_owned());
            fs::remove_file(bootstrap.join(missing)).unwrap();
            fs::write(
                bootstrap.join("bootstrap.json"),
                r#"{"version":"v0.1.27","target":"x86_64-pc-windows-msvc"}"#,
            )
            .unwrap();
            let root = temporary.path().join("runtime-root");
            assert!(
                activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).is_err(),
                "a bootstrap without {missing} must not be activated"
            );
            assert!(!root.join("current.json").exists());
        }
    }
}

#[cfg(not(windows))]
#[test]
fn activation_refuses_a_bootstrap_without_its_packaged_runtime_or_authority() {
    for missing in [
        "runtime/node/bin/node",
        "runtime/claurst/claurst",
        "runtime/claurst/llama/llama-server",
        "authority/ordinary-authority.json",
        "authority/root-keys.json",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let bootstrap = temporary.path().join("bootstrap");
        write_bootstrap(&bootstrap, |name| name.to_owned());
        fs::remove_file(bootstrap.join(missing)).unwrap();
        fs::write(
            bootstrap.join("bootstrap.json"),
            format!(r#"{{"version":"v0.1.27","target":"{TARGET}"}}"#),
        )
        .unwrap();
        let root = temporary.path().join("runtime-root");
        assert!(
            activate_at(&bootstrap, &root, temporary.path(), verify_scheduler).is_err(),
            "a bootstrap without {missing} must not be activated"
        );
        assert!(!root.join("current").exists());
    }
}
