use std::{fs, io::Write, path::Path};

use super::{
    display, files,
    scheduling::auto_update_helper_name,
    version::{is_generation, valid_release},
};

pub(super) fn selected_release(root: &Path) -> Result<Option<String>, String> {
    #[cfg(windows)]
    let release = {
        let path = root.join("current.json");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(display(error)),
        };
        if !metadata.is_file() || files::redirected(&metadata) || metadata.len() > 4096 {
            return Err("Gent current runtime pointer is invalid".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(path).map_err(display)?).map_err(display)?;
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or("Gent current runtime pointer schema is invalid")?;
        object
            .get("release")
            .and_then(serde_json::Value::as_str)
            .ok_or("Gent current runtime pointer is invalid")?
            .to_owned()
    };
    #[cfg(not(windows))]
    let release = {
        let path = root.join("current");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(display(error)),
        };
        if !metadata.file_type().is_symlink() {
            return Err("Gent current runtime pointer is not a symlink".into());
        }
        let target = fs::read_link(path).map_err(display)?;
        let release = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Gent current runtime pointer is invalid")?
            .to_owned();
        let expected = Path::new("releases").join(&release);
        if target != expected && target != root.join(expected) {
            return Err("Gent current runtime pointer escaped its release store".into());
        }
        release
    };
    if !valid_release(&release, &super::expected_target()?) {
        return Err("Gent current runtime identity is invalid".into());
    }
    Ok(Some(release))
}

pub(super) fn select_release(root: &Path, release: &str) -> Result<(), String> {
    if !is_generation(release, &super::expected_target()?) {
        return Err("new runtime selections must use immutable generations".into());
    }
    #[cfg(windows)]
    {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).map_err(display)?;
        let launcher = root
            .join("releases")
            .join(release)
            .join("gent-launcher.exe");
        for name in ["gent.exe", "gentd.exe"] {
            replace_bytes(
                &fs::read(&launcher).map_err(display)?,
                &bin.join(name),
                false,
            )?;
        }
        replace_bytes(
            &serde_json::to_vec(&serde_json::json!({"release":release})).map_err(display)?,
            &root.join("current.json"),
            false,
        )
    }
    #[cfg(not(windows))]
    {
        let temporary = root.join(format!(".current-{}", uuid::Uuid::new_v4().simple()));
        std::os::unix::fs::symlink(Path::new("releases").join(release), &temporary)
            .map_err(display)?;
        let result = fs::rename(&temporary, root.join("current")).map_err(display);
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

pub(super) fn refresh_auto_update_helper(root: &Path) -> Result<(), String> {
    let release = selected_release(root)?.ok_or("Gent has no selected runtime")?;
    let source = root
        .join("releases")
        .join(release)
        .join(auto_update_helper_name());
    let metadata = fs::symlink_metadata(&source).map_err(display)?;
    if !metadata.is_file() || files::redirected(&metadata) {
        return Err("Gent automatic-update helper is invalid".into());
    }
    replace_bytes(
        &fs::read(source).map_err(display)?,
        &root.join(auto_update_helper_name()),
        true,
    )
}

fn replace_bytes(bytes: &[u8], destination: &Path, executable: bool) -> Result<(), String> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if !metadata.is_file() || files::redirected(&metadata) {
                return Err("runtime selection metadata is redirected".into());
            }
            if fs::read(destination).map_err(display)? == bytes {
                return Ok(());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(display(error)),
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        destination
            .parent()
            .ok_or("runtime selection has no parent")?,
    )
    .map_err(display)?;
    temporary.write_all(bytes).map_err(display)?;
    temporary.as_file().sync_all().map_err(display)?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(display)?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    temporary.persist(destination).map_err(|error| {
        format!(
            "runtime selection cannot replace a busy shared file: {}",
            error.error
        )
    })?;
    Ok(())
}

#[cfg(test)]
pub(super) fn active_daemon(root: &Path) -> std::path::PathBuf {
    let directory = selected_release(root).ok().flatten().map_or_else(
        || root.join("current"),
        |release| root.join("releases").join(release),
    );
    directory.join(if cfg!(windows) { "gentd.exe" } else { "gentd" })
}
