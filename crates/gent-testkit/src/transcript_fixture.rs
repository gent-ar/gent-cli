//! Safe fixture-path resolution and loading for transcript validators.

use std::path::Path;

use crate::{PublicDriverFixture, load_public_driver_fixture};

pub(super) fn load_fixture(
    root: &Path,
    relative: Option<&Path>,
    vendor: &str,
    scenario: &str,
) -> Result<PublicDriverFixture, String> {
    let Some(relative) = relative else {
        return Err(format!("fixture path is required for {vendor}/{scenario}"));
    };
    if relative
        .extension()
        .is_none_or(|extension| extension != "jsonl")
        || relative.is_absolute()
        || relative.components().any(|part| part.as_os_str() == "..")
    {
        return Err(format!(
            "fixture path must be repository-relative for {vendor}/{scenario}"
        ));
    }
    let root = std::fs::canonicalize(root).map_err(|error| format!("invalid root: {error}"))?;
    let path = std::fs::canonicalize(root.join(relative))
        .map_err(|error| format!("could not resolve fixture: {error}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("fixture must be a regular file inside the manifest directory".into());
    }
    load_public_driver_fixture(path).map_err(|error| error.to_string())
}
