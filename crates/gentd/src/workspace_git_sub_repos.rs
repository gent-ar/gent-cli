use std::path::Path;

pub fn discover(root: &Path) -> Result<Vec<String>, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    if root.join(".git").exists() {
        return Ok(Vec::new());
    }
    let mut entries = std::fs::read_dir(&root)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::path);
    let mut sub_repos = Vec::new();
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if path.join(".git").exists() {
            sub_repos.push(path.display().to_string());
        }
    }
    Ok(sub_repos)
}

#[cfg(test)]
mod tests {
    use super::discover;

    #[test]
    fn a_repository_root_reports_no_sub_repos_of_itself() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".git")).unwrap();
        assert!(discover(directory.path()).unwrap().is_empty());
    }

    #[test]
    fn nested_git_directories_one_level_down_are_reported_sorted() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["b-repo", "a-repo", "not-a-repo"] {
            std::fs::create_dir(directory.path().join(name)).unwrap();
        }
        std::fs::create_dir(directory.path().join("a-repo").join(".git")).unwrap();
        std::fs::create_dir(directory.path().join("b-repo").join(".git")).unwrap();
        let sub_repos = discover(directory.path()).unwrap();
        assert_eq!(sub_repos.len(), 2);
        assert!(sub_repos[0].ends_with("a-repo"));
        assert!(sub_repos[1].ends_with("b-repo"));
    }
}
