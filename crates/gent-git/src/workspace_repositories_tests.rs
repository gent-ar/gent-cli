use std::{path::Path, process::Command};

use super::workspace_repositories;

fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args([
            "-c",
            "protocol.file.allow=always",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn repository(directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    git(directory, &["init", "--quiet"]);
    std::fs::write(directory.join("README.md"), "readme").unwrap();
    git(directory, &["add", "README.md"]);
    git(directory, &["commit", "--quiet", "-m", "init"]);
}

fn canonical(path: &Path) -> String {
    path.canonicalize().unwrap().display().to_string()
}

fn git_reported_nested(root: &Path) -> Vec<String> {
    let mut nested = git(root, &["status", "--porcelain=v1", "--untracked-files=all"])
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .map(|path| root.join(path.trim_end_matches('/')))
        .filter(|path| path.join(".git").exists())
        .map(|path| canonical(&path))
        .chain(
            git(root, &["submodule", "status"])
                .lines()
                .filter_map(|line| line.split_whitespace().nth(1))
                .map(|path| canonical(&root.join(path))),
        )
        .collect::<Vec<_>>();
    nested.sort();
    nested
}

#[test]
fn a_repository_with_nested_repositories_lists_itself_first_then_each_nested_repository() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    repository(&root);
    repository(&root.join("sub"));
    repository(&root.join("packages/deep/inner"));
    repository(&directory.path().join("library"));
    git(
        &root,
        &[
            "submodule",
            "add",
            "--quiet",
            &canonical(&directory.path().join("library")),
            "vendor/library",
        ],
    );
    git(&root, &["commit", "--quiet", "-m", "add submodule"]);
    git(
        &root,
        &["worktree", "add", "--quiet", "linked", "-b", "feature"],
    );
    let outside = directory.path().join("outside-worktree");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            outside.to_str().unwrap(),
            "-b",
            "outside",
        ],
    );
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    repository(&root.join(".hidden/ignored"));

    let reported = workspace_repositories(&root).unwrap();

    let mut expected = git_reported_nested(&root);
    expected.retain(|path| !path.ends_with("/linked") && !path.contains("/.hidden/"));
    expected.insert(0, canonical(&root));
    assert_eq!(reported, expected);
    assert_eq!(
        reported,
        vec![
            canonical(&root),
            canonical(&root.join("packages/deep/inner")),
            canonical(&root.join("sub")),
            canonical(&root.join("vendor/library")),
        ]
    );
}

#[test]
fn a_repository_without_nested_repositories_is_presented_as_one_repository() {
    let directory = tempfile::tempdir().unwrap();
    repository(directory.path());
    git(
        directory.path(),
        &["worktree", "add", "--quiet", "linked", "-b", "feature"],
    );
    assert!(workspace_repositories(directory.path()).unwrap().is_empty());
}

#[test]
fn a_folder_of_repositories_lists_only_the_repositories() {
    let directory = tempfile::tempdir().unwrap();
    repository(&directory.path().join("b-repo"));
    repository(&directory.path().join("a-repo"));
    std::fs::create_dir(directory.path().join("not-a-repo")).unwrap();
    assert_eq!(
        workspace_repositories(directory.path()).unwrap(),
        vec![
            canonical(&directory.path().join("a-repo")),
            canonical(&directory.path().join("b-repo")),
        ]
    );
}
