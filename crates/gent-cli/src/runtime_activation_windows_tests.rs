use super::*;

fn bootstrap(root: &Path, version: &str, helper: &str) {
    for name in required_files() {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, name).unwrap();
    }
    fs::write(root.join(auto_update_helper_name()), helper).unwrap();
    fs::write(
        root.join("bootstrap.json"),
        serde_json::json!({"version": version, "target": "x86_64-pc-windows-msvc"}).to_string(),
    )
    .unwrap();
}

#[test]
fn windows_activation_refreshes_the_selected_release_helper() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("bootstrap");
    let root = temporary.path().join("runtime");
    let data = temporary.path().join("data");
    bootstrap(&source, "v0.2.3", "new helper");
    for _ in 0..2 {
        let daemon = activate(source.clone(), Some(root.clone()), data.clone()).unwrap();
        assert_eq!(daemon, root.join("bin/gentd.exe"));
        assert!(daemon.is_file());
        assert_eq!(
            fs::read_to_string(root.join(auto_update_helper_name())).unwrap(),
            "new helper"
        );
        assert!(!root.join("current").exists());
    }
}

#[test]
fn windows_activation_preserves_a_newer_selected_runtime() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("bootstrap");
    let root = temporary.path().join("runtime");
    let data = temporary.path().join("data");
    bootstrap(&source, "v0.2.4", "newer helper");
    activate(source.clone(), Some(root.clone()), data.clone()).unwrap();
    bootstrap(&source, "v0.2.3", "older helper");
    activate(source, Some(root.clone()), data).unwrap();
    assert_eq!(
        selected_release(&root).unwrap().as_deref(),
        Some("v0.2.4-x86_64-pc-windows-msvc")
    );
    assert_eq!(
        fs::read_to_string(root.join(auto_update_helper_name())).unwrap(),
        "newer helper"
    );
}
