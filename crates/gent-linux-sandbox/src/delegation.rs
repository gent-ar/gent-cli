//! Read-only discovery for a service started with systemd DelegateSubgroup.
//! Never moves the caller or enables an existing ancestor's controllers.
use std::path::PathBuf;
pub fn current_service_parent(expected_leaf: &str) -> Result<PathBuf, &'static str> {
    if expected_leaf.is_empty()
        || expected_leaf.len() > 64
        || !expected_leaf
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid service subgroup");
    }
    let membership = std::fs::read_to_string("/proc/self/cgroup")
        .map_err(|_| "cgroup membership unavailable")?;
    let lines = membership.lines().collect::<Vec<_>>();
    if lines.len() != 1 {
        return Err("unified cgroup membership required");
    }
    let relative = lines[0]
        .strip_prefix("0::/")
        .ok_or("unified cgroup membership required")?;
    let current = PathBuf::from("/sys/fs/cgroup").join(relative);
    if current.file_name().and_then(|p| p.to_str()) != Some(expected_leaf) {
        return Err("service delegation subgroup missing");
    }
    let parent = current
        .parent()
        .ok_or("service delegation parent missing")?
        .to_owned();
    gent_linux_cgroup_owner::DelegatedParent::inspect(&parent)
        .map_err(|_| "nonroot memory/pids delegation unavailable")?;
    Ok(parent)
}
#[cfg(test)]
mod tests {
    #[test]
    fn invalid_subgroup_never_selects_arbitrary_path() {
        for leaf in ["", "..", "../parent", "a/b", "a.b"] {
            assert!(super::current_service_parent(leaf).is_err());
        }
    }
}

/// Explicit startup bootstrap for a service already in its `controller` subgroup.
pub fn prepare_current_service() -> Result<(), &'static str> {
    gent_linux_cgroup_owner::prepare_current_service()
        .map_err(|_| "own nonroot service memory/pids delegation preparation failed")
}
