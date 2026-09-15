use std::path::Path;

pub(crate) fn has_repository_marker(directory: &Path) -> bool {
    directory.ancestors().any(|ancestor| {
        ancestor.join(".git").exists()
            || (ancestor.join("HEAD").is_file()
                && ancestor.join("objects").is_dir()
                && ancestor.join("refs").exists())
    })
}
