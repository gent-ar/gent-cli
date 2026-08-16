//! Signed Windows dispatcher for immutable Gent release pairs.
//!
//! The installer places identical copies at `bin/gent.exe` and `bin/gentd.exe`.
//! It deliberately uses `Command` with `OsString` arguments, never a shell.

#[cfg(not(windows))]
fn main() {
    eprintln!("gent-launcher is only used by the Windows installer");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = launch() {
        eprintln!("Gent launcher failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn launch() -> Result<(), String> {
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Command;

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let binary = executable
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("launcher executable name is invalid")?;
    if binary != "gent" && binary != "gentd" {
        return Err("launcher must be named gent.exe or gentd.exe".into());
    }
    let root = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("launcher installation root is invalid")?;
    let release = read_release(&root.join("current.json"))?;
    let releases = root.join("releases");
    assert_plain_directory(&releases, "release directory")?;
    let selected = releases.join(release);
    assert_plain_directory(&selected, "selected release")?;
    let target = selected.join(format!("{binary}.exe"));
    assert_plain_file(&target, "selected release binary")?;
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let status = Command::new(target)
        .args(arguments)
        .status()
        .map_err(|error| format!("could not start selected release: {error}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(windows)]
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

#[cfg(windows)]
fn assert_plain_file(path: &std::path::Path, name: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(format!("{name} is not a regular file"));
    }
    Ok(())
}

#[cfg(windows)]
fn assert_plain_directory(path: &std::path::Path, name: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(format!("{name} is not a regular directory"));
    }
    Ok(())
}

#[cfg(windows)]
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

#[cfg(all(test, windows))]
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
