use std::{fs, process::Command};

use gent_types::WorkspaceFileChange;

use super::{MAX_CHANGED_FILES, changes_since, parse_numstat_z, snapshot_tree};

fn git(directory: &std::path::Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args([
                "-c",
                "user.name=Gent",
                "-c",
                "user.email=gent@example.invalid"
            ])
            .args(args)
            .current_dir(directory)
            .status()
            .unwrap()
            .success()
    );
}

fn change(path: &str, added: u64, removed: u64) -> WorkspaceFileChange {
    WorkspaceFileChange {
        path: path.into(),
        added,
        removed,
    }
}

#[test]
fn counts_only_what_changed_after_the_baseline_including_files_written_by_a_shell() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    git(&root, &["init", "--quiet"]);
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    fs::write(root.join("kept.txt"), "one\ntwo\nthree\n").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    fs::write(root.join("dirty-before.txt"), "the user's own edit\n").unwrap();
    let workspace = root.join("app");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("existing.txt"), "a\nb\n").unwrap();

    let baseline = snapshot_tree(&workspace).unwrap();
    fs::write(workspace.join("existing.txt"), "a\nB\nc\n").unwrap();
    fs::write(workspace.join("heredoc.txt"), "1\n2\n3\n4\n").unwrap();
    fs::write(workspace.join("image.bin"), [0_u8, 159, 146, 150, 0, 1]).unwrap();
    fs::create_dir(workspace.join("build")).unwrap();
    fs::write(workspace.join("build").join("out.txt"), "ignored\n").unwrap();
    fs::write(root.join("outside.txt"), "outside the workspace\n").unwrap();

    let changes = changes_since(&workspace, &baseline).unwrap();
    assert_eq!(
        changes.files,
        vec![change("existing.txt", 2, 1), change("heredoc.txt", 4, 0)]
    );
    assert!(!changes.truncated);
    assert!(
        git_status_is_untouched(&root),
        "the user's index must not change"
    );
}

fn git_status_is_untouched(root: &std::path::Path) -> bool {
    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(root)
        .output()
        .unwrap();
    staged.status.success() && staged.stdout.is_empty()
}

#[test]
fn a_folder_that_is_not_a_git_work_tree_has_no_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    assert!(snapshot_tree(&directory.path().canonicalize().unwrap()).is_err());
}

#[test]
fn a_baseline_that_is_not_an_object_id_is_refused_before_git_runs() {
    let directory = tempfile::tempdir().unwrap();
    assert!(changes_since(directory.path(), "HEAD --output=x").is_err());
}

#[test]
fn caps_the_file_list_and_skips_binary_entries() {
    let mut output = b"-\t-\tlogo.png\0".to_vec();
    for index in 0..=MAX_CHANGED_FILES {
        output.extend_from_slice(format!("1\t0\tfile-{index}.txt\0").as_bytes());
    }
    let parsed = parse_numstat_z(&output).unwrap();
    assert_eq!(parsed.files.len(), MAX_CHANGED_FILES);
    assert_eq!(parsed.files[0], change("file-0.txt", 1, 0));
    assert!(parsed.truncated);
}
