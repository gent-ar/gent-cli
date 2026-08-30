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
    fs::create_dir_all(&bootstrap).unwrap();
    for name in required_files() {
        fs::write(bootstrap.join(name), name).unwrap();
    }
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
    fs::create_dir_all(&bootstrap).unwrap();
    for name in required_files() {
        fs::write(bootstrap.join(name), name).unwrap();
    }
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
fn activation_refreshes_the_runtime_root_auto_update_helper() {
    let temporary = tempfile::tempdir().unwrap();
    let bootstrap = temporary.path().join("bootstrap");
    fs::create_dir_all(&bootstrap).unwrap();
    for name in required_files() {
        fs::write(bootstrap.join(name), format!("new-{name}")).unwrap();
    }
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
    fs::create_dir_all(&bootstrap).unwrap();
    for name in required_files() {
        fs::write(bootstrap.join(name), name).unwrap();
    }
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
