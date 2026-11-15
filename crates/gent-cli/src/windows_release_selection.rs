use std::path::{Path, PathBuf};

pub(crate) fn selected_binary(root: &Path, binary: &str) -> Result<PathBuf, String> {
    let release = read_release(&root.join("current.json"))?;
    let releases = root.join("releases");
    assert_plain_directory(&releases, "release directory")?;
    let selected = releases.join(release);
    assert_plain_directory(&selected, "selected release")?;
    let target = selected.join(format!("{binary}.exe"));
    assert_plain_file(&target, "selected release binary")?;
    Ok(target)
}

fn read_release(pointer: &std::path::Path) -> Result<String, String> {
    assert_plain_file(pointer, "current pointer")?;
    let text = std::fs::read_to_string(pointer).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or("current pointer is not an object")?;
    if object.len() != 1 {
        return Err("current pointer has an unexpected schema".into());
    }
    let release = object
        .get("release")
        .and_then(serde_json::Value::as_str)
        .ok_or("current pointer has no release")?;
    if !is_release_name(release) {
        return Err("current pointer has an invalid release identity".into());
    }
    Ok(release.to_owned())
}

fn assert_plain_file(path: &std::path::Path, name: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(format!("{name} is not a regular file"));
    }
    Ok(())
}

fn assert_plain_directory(path: &std::path::Path, name: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(format!("{name} is not a regular directory"));
    }
    Ok(())
}

fn is_release_name(value: &str) -> bool {
    let suffix = "-x86_64-pc-windows-msvc";
    let Some(version) = value.strip_suffix(suffix) else {
        return false;
    };
    let Some(version) = version.strip_prefix('v') else {
        return false;
    };
    let qualifier_at = version.find(['-', '+']).unwrap_or(version.len());
    let (core, qualifier) = version.split_at(qualifier_at);
    let numeric = core.split('.').collect::<Vec<_>>();
    numeric.len() == 3
        && numeric
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && (qualifier.is_empty()
            || (qualifier.len() > 1
                && matches!(qualifier.as_bytes()[0], b'-' | b'+')
                && qualifier.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-')
                })))
}

#[cfg(test)]
mod tests {
    use super::is_release_name;

    #[test]
    fn accepts_only_safe_windows_release_names() {
        assert!(is_release_name("v1.2.3-x86_64-pc-windows-msvc"));
        assert!(is_release_name("v1.2.3-rc.1-x86_64-pc-windows-msvc"));
        assert!(!is_release_name("v1.2.3/../../bad-x86_64-pc-windows-msvc"));
        assert!(!is_release_name("v1.2.3-x86_64-pc-windows-msvc.extra"));
    }
}
