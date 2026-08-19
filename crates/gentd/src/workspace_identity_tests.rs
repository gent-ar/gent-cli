use crate::workspace_identity::{CanonicalWorkspace, WorkspaceIdentityError};

#[test]
fn canonical_workspace_identity_is_stable_across_equivalent_paths() {
    let root = tempfile::tempdir().unwrap();
    let first = CanonicalWorkspace::from_path(root.path()).unwrap();
    let second = CanonicalWorkspace::from_path(&root.path().join(".")).unwrap();

    assert_eq!(first.record(), second.record());
    assert!(first.record().workspace_id.starts_with("workspace-"));
    assert_eq!(first.record().workspace_id.len(), "workspace-".len() + 64);
}

#[test]
fn a_file_cannot_become_a_workspace() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    std::fs::write(&file, "not a workspace").unwrap();

    assert_eq!(
        CanonicalWorkspace::from_path(&file),
        Err(WorkspaceIdentityError::NotDirectory)
    );
}
