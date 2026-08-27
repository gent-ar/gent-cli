use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackagedClaurstRuntime {
    pub(crate) claurst_executable: PathBuf,
    pub(crate) llama_server_executable: PathBuf,
}

impl PackagedClaurstRuntime {
    pub(crate) fn from_current_executable() -> Result<Option<Self>, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not locate Gent executable: {error}"))?;
        Ok(Self::from_gentd_executable(&executable))
    }

    pub(crate) fn from_gentd_executable(gentd_executable: &Path) -> Option<Self> {
        runtime_directories(gentd_executable).find_map(|directory| {
            let claurst_executable = directory.join(claurst_name());
            let llama_server_executable = directory.join("llama").join(llama_server_name());
            (claurst_executable.is_file() && llama_server_executable.is_file()).then_some(Self {
                claurst_executable,
                llama_server_executable,
            })
        })
    }
}

fn runtime_directories(gentd_executable: &Path) -> impl Iterator<Item = PathBuf> {
    let executable_directory = gentd_executable.parent().unwrap_or_else(|| Path::new(""));
    #[cfg(target_os = "macos")]
    let directories = [
        executable_directory.join("runtime/claurst"),
        executable_directory
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Resources/claurst"),
    ];
    #[cfg(target_os = "linux")]
    let directories = [
        executable_directory.join("runtime/claurst"),
        executable_directory.join("data/claurst"),
        executable_directory
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("data/claurst"),
    ];
    #[cfg(windows)]
    let directories = [
        executable_directory.join("runtime/claurst"),
        executable_directory.join("data/claurst"),
    ];
    directories.into_iter()
}

#[cfg(windows)]
const fn claurst_name() -> &'static str {
    "claurst.exe"
}

#[cfg(not(windows))]
const fn claurst_name() -> &'static str {
    "claurst"
}

#[cfg(windows)]
const fn llama_server_name() -> &'static str {
    "llama-server.exe"
}

#[cfg(not(windows))]
const fn llama_server_name() -> &'static str {
    "llama-server"
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{PackagedClaurstRuntime, claurst_name, llama_server_name};

    #[test]
    fn resolves_the_native_app_bundle_layout() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("bin/gentd");
        let runtime = native_runtime_directory(root.path());
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join(claurst_name()), "claurst").unwrap();
        fs::create_dir_all(runtime.join("llama")).unwrap();
        fs::write(
            runtime.join("llama").join(llama_server_name()),
            "llama-server",
        )
        .unwrap();
        let found = PackagedClaurstRuntime::from_gentd_executable(&executable).unwrap();
        assert_eq!(found.claurst_executable, runtime.join(claurst_name()));
        assert_eq!(
            found.llama_server_executable,
            runtime.join("llama").join(llama_server_name())
        );
    }

    #[test]
    fn resolves_the_installed_release_runtime_layout() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("release/gentd");
        let runtime = root.path().join("release/runtime/claurst");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join(claurst_name()), "claurst").unwrap();
        fs::create_dir_all(runtime.join("llama")).unwrap();
        fs::write(
            runtime.join("llama").join(llama_server_name()),
            "llama-server",
        )
        .unwrap();
        let found = PackagedClaurstRuntime::from_gentd_executable(&executable).unwrap();
        assert_eq!(found.claurst_executable, runtime.join(claurst_name()));
        assert_eq!(
            found.llama_server_executable,
            runtime.join("llama").join(llama_server_name())
        );
    }

    #[test]
    fn refuses_a_partial_packaged_runtime() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("bin/gentd");
        let runtime = native_runtime_directory(root.path());
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join(claurst_name()), "claurst").unwrap();
        assert!(PackagedClaurstRuntime::from_gentd_executable(&executable).is_none());
    }

    #[cfg(target_os = "macos")]
    fn native_runtime_directory(root: &Path) -> std::path::PathBuf {
        root.join("Resources/claurst")
    }

    #[cfg(target_os = "linux")]
    fn native_runtime_directory(root: &Path) -> std::path::PathBuf {
        root.join("bin/data/claurst")
    }

    #[cfg(windows)]
    fn native_runtime_directory(root: &Path) -> std::path::PathBuf {
        root.join("bin/data/claurst")
    }
}
