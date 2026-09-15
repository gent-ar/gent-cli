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

pub(super) fn required_files() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "gent.exe",
            "gentd.exe",
            "gent-launcher.exe",
            "gent-auto-update.ps1",
            "runtime/node/bin/node.exe",
            "runtime/node/bin/npm.cmd",
            "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
            "runtime/claurst/claurst.exe",
            "runtime/claurst/llama/llama-server.exe",
            "authority/ordinary-authority.json",
            "authority/root-keys.json",
        ]
    }
    #[cfg(not(windows))]
    {
        &[
            "gent",
            "gentd",
            "gent-auto-update.py",
            "runtime/node/bin/node",
            "runtime/node/bin/npm",
            "runtime/node/lib/node_modules/npm/bin/npm-cli.js",
            "runtime/claurst/claurst",
            "runtime/claurst/llama/llama-server",
            "authority/ordinary-authority.json",
            "authority/root-keys.json",
        ]
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
