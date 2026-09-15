use std::path::{Path, PathBuf};

use serde::Deserialize;

const MAX_ROOT_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackagedAuthority {
    pub(crate) release: PathBuf,
    pub(crate) root_keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootKeyMetadata {
    version: u16,
    keys: Vec<String>,
}

impl PackagedAuthority {
    pub(crate) fn from_current_executable() -> Result<Option<Self>, String> {
        let executable = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .map_err(|error| format!("could not locate Gent executable: {error}"))?;
        Self::from_gentd_executable(&executable)
    }

    pub(crate) fn from_gentd_executable(gentd_executable: &Path) -> Result<Option<Self>, String> {
        let directory = gentd_executable
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("authority");
        let release = directory.join("ordinary-authority.json");
        let roots = directory.join("root-keys.json");
        match (regular_file(&release), regular_file(&roots)) {
            (false, false) => Ok(None),
            (true, true) => Ok(Some(Self {
                release,
                root_keys: root_keys(&roots)?,
            })),
            _ => Err("packaged Gent authority is incomplete".into()),
        }
    }
}

fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn root_keys(path: &Path) -> Result<Vec<String>, String> {
    let invalid = || "packaged Gent authority root keys are invalid".to_owned();
    if std::fs::metadata(path).map_err(|_| invalid())?.len() > MAX_ROOT_METADATA_BYTES {
        return Err(invalid());
    }
    let metadata: RootKeyMetadata =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    (metadata.version == 1)
        .then_some(metadata.keys)
        .ok_or_else(invalid)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::PackagedAuthority;

    #[test]
    fn resolves_the_authority_packaged_beside_gentd() {
        let root = tempfile::tempdir().unwrap();
        let authority = root.path().join("release/authority");
        fs::create_dir_all(&authority).unwrap();
        fs::write(authority.join("ordinary-authority.json"), "{}").unwrap();
        fs::write(
            authority.join("root-keys.json"),
            format!(r#"{{"version":1,"keys":["root:{}"]}}"#, "01".repeat(32)),
        )
        .unwrap();
        let found = PackagedAuthority::from_gentd_executable(&root.path().join("release/gentd"))
            .unwrap()
            .unwrap();
        assert_eq!(found.release, authority.join("ordinary-authority.json"));
        assert_eq!(found.root_keys, [format!("root:{}", "01".repeat(32))]);
    }

    #[test]
    fn a_release_without_authority_has_none() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            PackagedAuthority::from_gentd_executable(&root.path().join("gentd")).unwrap(),
            None
        );
    }

    #[test]
    fn partial_or_malformed_authority_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let authority = root.path().join("authority");
        fs::create_dir_all(&authority).unwrap();
        fs::write(authority.join("ordinary-authority.json"), "{}").unwrap();
        let gentd = root.path().join("gentd");
        assert!(PackagedAuthority::from_gentd_executable(&gentd).is_err());
        fs::write(
            authority.join("root-keys.json"),
            r#"{"version":2,"keys":[]}"#,
        )
        .unwrap();
        assert!(PackagedAuthority::from_gentd_executable(&gentd).is_err());
    }
}
