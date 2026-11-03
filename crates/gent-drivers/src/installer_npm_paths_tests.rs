use super::*;

#[test]
fn checked_existing_and_missing_paths_preserve_identity_without_creating_files() {
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let rendered = checked_filesystem_path(&canonical).unwrap();
    assert_eq!(Path::new(&rendered).canonicalize().unwrap(), canonical);
    let missing = canonical.join("not-created").join(".npmrc");
    assert_eq!(
        checked_filesystem_path(&missing).unwrap(),
        filesystem_path(&missing)
    );
    assert!(!missing.parent().unwrap().exists());
}

#[cfg(windows)]
#[test]
fn unsafe_nonexistent_win32_names_are_rejected_before_consumer_reinterpretation() {
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    for name in [
        "trailing.",
        "trailing ",
        "CON",
        "NUL.txt",
        "COM1",
        "LPT9.cfg",
    ] {
        assert!(
            checked_filesystem_path(&canonical.join(name)).is_err(),
            "{name}"
        );
    }
}

#[cfg(windows)]
#[test]
fn unc_install_prefix_is_refused_without_any_filesystem_lookup() {
    for prefix in [
        r"\\server\share\gent\npm-global",
        r"\\?\UNC\server\share\gent\npm-global",
    ] {
        assert_eq!(
            require_supported_prefix(Path::new(prefix)),
            Err(InstallerError::UnsupportedPrefix)
        );
        assert_eq!(
            filesystem_path(Path::new(prefix)),
            r"\\server\share\gent\npm-global"
        );
    }
}
