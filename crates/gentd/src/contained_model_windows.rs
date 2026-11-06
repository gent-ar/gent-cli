use super::ModelError;
use gent_drivers::supervisor::{LaunchIntent, ProviderLaunch};
use gent_types::{
    RunVersionLock, SandboxLaunchProfile, SandboxPermissionBinding, SandboxResourceGrant,
    SandboxResourceLimits, SandboxWorkspaceAccess,
};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: u32,
    transport: String,
    worker_sha256: String,
    helper_sha256: String,
    os_libraries: Vec<OsFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OsFile {
    name: String,
    sha256: String,
}
pub(crate) struct WindowsModelFactory {
    helper: RunVersionLock,
    worker: RunVersionLock,
    paths: Vec<PathBuf>,
    pins: Vec<File>,
}
pub(crate) struct SelectedModel {
    record: crate::local_model_catalog::LocalModelRecord,
    path: PathBuf,
    pin: File,
}
pub(crate) struct ModelRun {
    pub binding: SandboxPermissionBinding,
    pub sdk_lock: RunVersionLock,
    pub current: Arc<dyn Fn() -> bool + Send + Sync>,
    pub lifetime: Vec<Box<dyn Send>>,
    pub writable_roots: Vec<PathBuf>,
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn no_reparse(path: &Path) -> Result<PathBuf, ModelError> {
    if !path.is_absolute() {
        return Err(ModelError::Request);
    }
    for item in path.ancestors() {
        if std::fs::symlink_metadata(item)
            .map_err(|_| ModelError::Request)?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err(ModelError::Request);
        }
    }
    path.canonicalize().map_err(|_| ModelError::Request)
}
fn pin(path: &Path, expected: &str) -> Result<(PathBuf, File), ModelError> {
    if !digest(expected) {
        return Err(ModelError::Request);
    }
    let path = no_reparse(path)?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(&path)
        .map_err(|_| ModelError::Request)?;
    if !file.metadata().map_err(|_| ModelError::Request)?.is_file() {
        return Err(ModelError::Request);
    }
    let digest = gent_drivers::sha256_held(&path, &file).map_err(|_| ModelError::Request)?;
    if hex::encode(digest) != expected {
        return Err(ModelError::Request);
    }
    Ok((path, file))
}
impl SelectedModel {
    pub(crate) fn capture(
        record: crate::local_model_catalog::LocalModelRecord,
        path: &Path,
    ) -> Result<Self, ModelError> {
        let (path, pin) = pin(path, &record.sha256)?;
        if pin.metadata().map_err(|_| ModelError::Request)?.len() != record.size_bytes
            || !(4096..=32768).contains(&record.context_tokens)
            || record.runtime_memory_bytes <= record.size_bytes
        {
            return Err(ModelError::Request);
        }
        Ok(Self { record, path, pin })
    }
}
impl WindowsModelFactory {
    pub(crate) fn packaged() -> Result<Self, ModelError> {
        let expected = option_env!("GENT_CONTAINED_MODEL_MANIFEST_SHA256")
            .filter(|v| digest(v))
            .ok_or(ModelError::Request)?;
        let exe = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .map_err(|_| ModelError::Request)?;
        let runtime = exe.parent().ok_or(ModelError::Request)?.join("runtime");
        let manifest_path = runtime.join("claurst/model-server/model-runtime.json");
        let (_, mut manifest_pin) = pin(&manifest_path, expected)?;
        if manifest_pin
            .metadata()
            .map_err(|_| ModelError::Request)?
            .len()
            > 16384
        {
            return Err(ModelError::Request);
        }
        let mut source = String::new();
        manifest_pin
            .read_to_string(&mut source)
            .map_err(|_| ModelError::Request)?;
        let manifest: Manifest = serde_json::from_str(&source).map_err(|_| ModelError::Request)?;
        if manifest.schema != 1
            || manifest.transport != "server-context-stdio-v1"
            || manifest.os_libraries.is_empty()
            || manifest.os_libraries.len() > 64
        {
            return Err(ModelError::Request);
        }
        let native_helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()
            .map_err(|_| ModelError::Request)?;
        super::admission::verify_helper_manifest(
            &manifest.helper_sha256,
            &native_helper.image_lock().digest_sha256,
        )?;
        let (worker, worker_pin) = pin(
            &runtime.join("claurst/model-server/gent-contained-model-factory.exe"),
            &manifest.worker_sha256,
        )?;
        let (helper, helper_pin) = pin(
            &runtime.join("windows/provider-helper.exe"),
            &manifest.helper_sha256,
        )?;
        let worker_lock =
            gent_drivers::lock::capture("claurst-model", &worker, "1", "server-context-stdio-v1")
                .map_err(|_| ModelError::Request)?;
        let helper_lock = gent_drivers::lock::capture(
            "windows-sandbox-helper",
            &helper,
            "1",
            "windows-psec-job-v1",
        )
        .map_err(|_| ModelError::Request)?;
        if helper_lock.digest_sha256 != manifest.helper_sha256
            || worker_lock.digest_sha256 != manifest.worker_sha256
        {
            return Err(ModelError::Request);
        }
        let mut paths = vec![helper.clone(), worker.clone()];
        let mut pins = vec![manifest_pin, worker_pin, helper_pin];
        let names: Vec<String> = manifest
            .os_libraries
            .iter()
            .map(|row| row.name.clone())
            .collect();
        let os = crate::windows_os_resources::verify_os_libraries(&native_helper, &names)
            .map_err(|_| ModelError::Request)?;
        let system = os.system_directory().to_path_buf();
        pins.extend(os.into_handles());
        let mut names = std::collections::BTreeSet::new();
        for row in manifest.os_libraries {
            if row.name.len() < 5
                || row.name.len() > 128
                || std::path::Path::new(&row.name)
                    .extension()
                    .is_none_or(|extension| extension != "dll")
                || !row.name.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
                })
                || row.name.contains("..")
                || !names.insert(row.name.clone())
            {
                return Err(ModelError::Request);
            }
            let (path, file) = pin(&system.join(row.name), &row.sha256)?;
            paths.push(path);
            pins.push(file);
        }
        Ok(Self {
            helper: helper_lock,
            worker: worker_lock,
            paths,
            pins,
        })
    }
}
#[path = "contained_model_windows_launch.rs"]
mod launch;
