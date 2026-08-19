#![cfg(unix)]

use std::process::Command;

#[test]
fn default_browser_rejects_non_tty_before_daemon_autostart() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .arg("--data-dir")
        .arg(directory.path())
        .env("GENTD_BIN", directory.path().join("must-not-start"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
    assert!(!directory.path().join("gentd.sock").exists());
}
