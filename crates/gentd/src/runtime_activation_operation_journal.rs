use gent_protocol::runtime_activation::{
    ACTIVATION_MAINTENANCE_MAX_BYTES, ActivationOperationRecord,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) fn directory(root: &Path, create: bool) -> Result<PathBuf, String> {
    let path = root.join("activation-operations");
    if create && !path.try_exists().map_err(|e| e.to_string())? {
        fs::create_dir(&path).map_err(|e| e.to_string())?;
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || super::super::files::redirected(&metadata)
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("activation journal directory is redirected".into());
    }
    Ok(path)
}
fn path(root: &Path, id: &str, create: bool) -> Result<PathBuf, String> {
    super::identity::operation_id(id)?;
    Ok(directory(root, create)?.join(format!("{id}.json")))
}
pub(super) fn read(root: &Path, id: &str) -> Result<ActivationOperationRecord, String> {
    let path = path(root, id, false)?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || super::super::files::redirected(&metadata)
        || metadata.len() > ACTIVATION_MAINTENANCE_MAX_BYTES as u64
    {
        return Err("invalid activation journal record".into());
    }
    let mut raw = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(ACTIVATION_MAINTENANCE_MAX_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > ACTIVATION_MAINTENANCE_MAX_BYTES {
        return Err("journal record exceeds bound".into());
    }
    let record: ActivationOperationRecord =
        serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    super::identity::validate(&record)?;
    if record.operation_id != id {
        return Err("journal operation ID mismatch".into());
    }
    Ok(record)
}
pub(super) fn write(record: &ActivationOperationRecord, first: bool) -> Result<(), String> {
    super::identity::validate(record)?;
    let path = path(&record.command.runtime_root, &record.operation_id, first)?;
    let raw = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if raw.len() > ACTIVATION_MAINTENANCE_MAX_BYTES {
        return Err("journal record exceeds bound".into());
    }
    if first {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&raw).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    } else {
        let old = read(&record.command.runtime_root, &record.operation_id)?;
        if old.command_sha256 != record.command_sha256 {
            return Err("operation identity replacement refused".into());
        }
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().ok_or("journal parent missing")?)
                .map_err(|e| e.to_string())?;
        file.write_all(&raw).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(&path).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or("journal parent missing")?)
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    Ok(())
}
