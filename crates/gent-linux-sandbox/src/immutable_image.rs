//! Admission for versioned system-managed images, including every path ancestor.
use std::{os::unix::fs::MetadataExt, path::Path};
pub(crate) fn require(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() || path.canonicalize().map_err(|_| "image path unavailable")? != path {
        return Err("image path redirected");
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|_| "image metadata unavailable")?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err("image is not system-managed immutable");
    }
    for ancestor in path.ancestors().skip(1) {
        let value = ancestor
            .symlink_metadata()
            .map_err(|_| "image ancestor unavailable")?;
        if !value.is_dir()
            || value.uid() != 0
            || value.mode() & 0o022 != 0 && value.mode() & 0o1000 == 0
        {
            return Err("image ancestor mutable");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn caller_owned_provider_image_cannot_be_promoted_by_a_digest() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("provider");
        std::fs::write(&image, b"owned QA image").unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            super::require(&image).is_err(),
            "the non-immutable temporary ancestor must be rejected even when this test runner is root"
        );
    }
}
