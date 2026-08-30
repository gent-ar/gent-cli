use std::{fs, io, path::Path};

pub(super) fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(display)?;
    for entry in fs::read_dir(source).map_err(display)? {
        let entry = entry.map_err(display)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(display)?;
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() && !metadata.file_type().is_symlink() {
            fs::copy(entry.path(), &target).map_err(display)?;
            fs::set_permissions(&target, metadata.permissions()).map_err(display)?;
        } else {
            return Err("Gent bootstrap contains an unsupported entry".into());
        }
    }
    Ok(())
}

pub(super) fn remove_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path).map_err(display),
        Ok(_) => fs::remove_file(path).map_err(display),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(display(error)),
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
