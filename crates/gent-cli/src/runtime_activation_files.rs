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

pub(super) fn same_tree(left: &Path, right: &Path) -> Result<bool, String> {
    let mut left_entries = fs::read_dir(left)
        .map_err(display)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(display)?;
    let mut right_entries = fs::read_dir(right)
        .map_err(display)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(display)?;
    left_entries.sort_by_key(|entry| entry.file_name());
    right_entries.sort_by_key(|entry| entry.file_name());
    if left_entries.len() != right_entries.len() {
        return Ok(false);
    }
    for (left, right) in left_entries.iter().zip(right_entries.iter()) {
        if left.file_name() != right.file_name() {
            return Ok(false);
        }
        let left_metadata = fs::symlink_metadata(left.path()).map_err(display)?;
        let right_metadata = fs::symlink_metadata(right.path()).map_err(display)?;
        if left_metadata.file_type().is_symlink() || right_metadata.file_type().is_symlink() {
            return Ok(false);
        }
        if left_metadata.is_dir() != right_metadata.is_dir()
            || left_metadata.is_file() != right_metadata.is_file()
        {
            return Ok(false);
        }
        if left_metadata.is_dir() {
            if !same_tree(&left.path(), &right.path())? {
                return Ok(false);
            }
        } else if fs::read(left.path()).map_err(display)?
            != fs::read(right.path()).map_err(display)?
        {
            return Ok(false);
        }
    }
    Ok(true)
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
