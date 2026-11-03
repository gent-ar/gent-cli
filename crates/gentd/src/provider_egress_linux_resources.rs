//! Required public gate image visibility, derived from installation authority.
//! This grants one exact already-selected image, never its parent directory.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HelperResourceError {
    Identity,
    Writable,
    Alias,
}
use gent_types::{RunVersionLock, SandboxResourceGrant};
use std::path::PathBuf;
pub(crate) fn helper_image(
    lock: &RunVersionLock,
) -> Result<SandboxResourceGrant, HelperResourceError> {
    if gent_drivers::lock::rechecked_identity(lock).map_err(|_| HelperResourceError::Identity)?
        != *lock
    {
        return Err(HelperResourceError::Identity);
    }
    Ok(SandboxResourceGrant::RuntimeReadOnly(PathBuf::from(
        &lock.canonical_path,
    )))
}
pub(crate) fn include_helper_image(
    resources: &mut Vec<SandboxResourceGrant>,
    lock: &RunVersionLock,
) -> Result<(), HelperResourceError> {
    let grant = helper_image(lock)?;
    let image = grant.root();
    for resource in resources.iter() {
        if resource.root().canonicalize().ok().as_deref() != Some(resource.root()) {
            return Err(HelperResourceError::Alias);
        }
    }
    if resources.iter().any(
        |r| matches!(r,SandboxResourceGrant::PrivateStateReadWrite(root)if image.starts_with(root)),
    ) {
        return Err(HelperResourceError::Writable);
    }
    if !resources.iter().any(|r|matches!(r,SandboxResourceGrant::RuntimeReadOnly(root)|SandboxResourceGrant::ConfigurationReadOnly(root)if image.starts_with(root))) {
        resources.push(grant);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_resource_alias_cannot_hide_writable_collision() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("images");
        std::fs::create_dir(&directory).unwrap();
        let image = directory.join("helper");
        std::fs::write(&image, b"image").unwrap();
        let lock = gent_drivers::lock::capture("helper", &image, "qa", "nonexecuted-resource-test")
            .unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&directory, &alias).unwrap();
        for grant in [
            SandboxResourceGrant::PrivateStateReadWrite(alias.clone()),
            SandboxResourceGrant::RuntimeReadOnly(alias),
        ] {
            let mut resources = vec![grant];
            assert_eq!(
                include_helper_image(&mut resources, &lock),
                Err(HelperResourceError::Alias)
            );
            assert_eq!(resources.len(), 1);
        }
    }
    #[test]
    fn exact_helper_resource_rechecks_identity_and_never_grants_parent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("helper");
        std::fs::write(&path, b"first").unwrap();
        let lock = gent_drivers::lock::capture("helper", &path, "qa", "nonexecuted-resource-test")
            .unwrap();
        assert!(
            matches!(helper_image(&lock).unwrap(),SandboxResourceGrant::RuntimeReadOnly(p)if p==path)
        );
        std::fs::write(&path, b"second-and-different").unwrap();
        assert!(helper_image(&lock).is_err());
    }
    #[test]
    fn helper_resource_is_idempotent_and_preserves_existing_readonly_coverage() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("helper");
        std::fs::write(&path, b"image").unwrap();
        let lock = gent_drivers::lock::capture("helper", &path, "qa", "nonexecuted-resource-test")
            .unwrap();
        let mut resources = Vec::new();
        include_helper_image(&mut resources, &lock).unwrap();
        include_helper_image(&mut resources, &lock).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].root(), path);
        let mut parent = vec![SandboxResourceGrant::RuntimeReadOnly(root.path().into())];
        include_helper_image(&mut parent, &lock).unwrap();
        assert_eq!(parent.len(), 1);
        assert_eq!(parent[0].root(), root.path());
        let mut writable = vec![SandboxResourceGrant::PrivateStateReadWrite(
            root.path().into(),
        )];
        assert!(include_helper_image(&mut writable, &lock).is_err());
        assert_eq!(writable.len(), 1);
    }
}
