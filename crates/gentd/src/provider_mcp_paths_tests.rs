use super::*;
#[test]
fn host_mcp_node_alias_swap_executes_only_the_pinned_canonical_entrypoint() {
    let root = tempfile::tempdir().unwrap();
    let host = root.path().join("host");
    let workspace = root.path().join("workspace");
    let worker = root.path().join("worker");
    for path in [&host, &workspace, &worker] {
        std::fs::create_dir(path).unwrap();
    }
    let target = host.join("server.js");
    let evil = workspace.join("evil.js");
    let alias = workspace.join("entry.js");
    std::fs::write(&target, "console.log('pinned-entrypoint')").unwrap();
    std::fs::write(&evil, "console.log('untrusted-entrypoint')").unwrap();
    std::os::windows::fs::symlink_file(&target, &alias).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let worker = worker.canonicalize().unwrap();
    let (entry, pins) = pin(&alias, &workspace, &worker).unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::windows::fs::symlink_file(&evil, &alias).unwrap();
    let output = std::process::Command::new("C:/Program Files/nodejs/node.exe")
        .arg(&entry)
        .env_clear()
        .env("SystemRoot", std::env::var_os("SystemRoot").unwrap())
        .current_dir(&host)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "node status {:?}",
        output.status.code()
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "pinned-entrypoint"
    );
    assert!(!pins.is_empty());
}
#[test]
fn host_mcp_writable_code_and_private_state_entrypoints_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let worker = root.path().join("worker");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&worker).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let worker = worker.canonicalize().unwrap();
    for directory in [&workspace, &worker] {
        let path = directory.join("server.js");
        std::fs::write(&path, b"// untrusted code").unwrap();
        assert!(pin(&path, &workspace, &worker).is_err());
    }
}

#[test]
#[cfg(windows)]
fn host_mcp_data_directory_uses_verified_ordinary_ipc_spelling() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("host-data");
    std::fs::create_dir(&data).unwrap();
    let workspace = root.path().join("workspace");
    let worker = root.path().join("worker");
    std::fs::create_dir(&worker).unwrap();
    let (argument, pins) = super::host_data(&data, &workspace, &worker).unwrap();
    assert!(!argument.to_str().unwrap().starts_with(r"\\?\"));
    assert_eq!(
        argument.canonicalize().unwrap(),
        data.canonicalize().unwrap()
    );
    assert!(!pins.is_empty());
    assert!(
        gent_types::windows_pipe_client_candidates(&argument)
            .contains(&gent_types::windows_pipe_name(&data))
    );
}

#[test]
fn host_mcp_installed_data_ancestor_is_host_only_and_worker_descendants_are_denied() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let worker = data.join("provider-state").join("private-run");
    let child = worker.join("untrusted-data");
    let workspace = root.path().join("workspace");
    for directory in [&data, &worker, &child, &workspace] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let worker = worker.canonicalize().unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let (argument, locks) = host_data(&data, &workspace, &worker).unwrap();
    assert_eq!(
        argument.canonicalize().unwrap(),
        data.canonicalize().unwrap()
    );
    assert!(!locks.is_empty());
    let ordinary_worker = std::path::PathBuf::from(
        worker
            .to_str()
            .unwrap()
            .strip_prefix(r"\\?\")
            .unwrap_or(worker.to_str().unwrap()),
    );
    for spelling in [&worker, &ordinary_worker] {
        assert!(host_data(&worker, &workspace, spelling).is_err());
        assert!(host_data(&child, &workspace, spelling).is_err());
        assert!(host_data(&data, &workspace, spelling).is_ok());
    }
    assert!(host_data(&worker, &workspace, &worker).is_err());
    assert!(host_data(&child, &workspace, &worker).is_err());
    assert!(host_data(&workspace, &workspace, &worker).is_err());
    assert!(host_data(root.path(), &workspace, &worker).is_err());
}

#[test]
#[cfg(windows)]
fn host_mcp_node_script_ordinary_argument_preserves_pinned_file_identity() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("script with spaces.js");
    std::fs::write(&source, b"// owned node entry").unwrap();
    let workspace = root.path().join("workspace");
    let worker = root.path().join("worker");
    let (canonical, pins) = super::pin(&source, &workspace, &worker).unwrap();
    let argument = super::ordinary_argument(&canonical).unwrap();
    assert!(!argument.to_str().unwrap().starts_with(r"\\?\"));
    assert_eq!(argument.canonicalize().unwrap(), canonical);
    assert_eq!(std::fs::read(&argument).unwrap(), b"// owned node entry");
    assert!(std::fs::remove_file(&source).is_err());
    assert!(super::ordinary_argument(&source).is_err());
    drop(pins);
    std::fs::remove_file(source).unwrap();
}
