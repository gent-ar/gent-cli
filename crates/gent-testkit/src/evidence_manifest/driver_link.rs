//! Cross-links authority evidence records to live public-driver transcripts.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use super::{non_empty, scalar};

#[derive(Debug, Deserialize)]
struct TranscriptManifest {
    cells: Vec<TranscriptCell>,
}

#[derive(Debug, Deserialize)]
struct TranscriptCell {
    state: String,
    path: Option<PathBuf>,
}

pub(super) fn manifest_path(
    root: &Mapping,
    manifest_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(value) = root.get(Value::String("public_driver_transcript_manifest".into())) else {
        return Ok(None);
    };
    let relative = PathBuf::from(scalar(value, "public_driver_transcript_manifest")?);
    if relative.is_absolute() || relative.components().any(|part| part.as_os_str() == "..") {
        return Err("public_driver_transcript_manifest must be repository-relative".into());
    }
    let root = std::fs::canonicalize(manifest_dir)
        .map_err(|error| format!("invalid coverage-manifest directory: {error}"))?;
    let path = std::fs::canonicalize(root.join(relative))
        .map_err(|error| format!("could not resolve public_driver_transcript_manifest: {error}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(
            "public_driver_transcript_manifest must be a regular file inside fixtures".into(),
        );
    }
    Ok(Some(path))
}

pub(super) fn validate_record_link(
    record: &Mapping,
    id: &str,
    coverage_root: &Path,
    transcript_manifest: Option<&Path>,
) -> Result<(), String> {
    let manifest = transcript_manifest.ok_or_else(|| {
        format!("authority transfer is blocked: evidence record {id} has no public-driver manifest")
    })?;
    crate::validate_public_driver_manifest(manifest, true)?;
    let transcript = record_path(record, id, coverage_root)?;
    let contents = std::fs::read_to_string(manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    let inventory: TranscriptManifest =
        serde_yaml::from_str(&contents).map_err(|error| format!("invalid YAML: {error}"))?;
    let inventory_root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let recorded = inventory.cells.iter().any(|cell| {
        cell.state == "recorded"
            && cell.path.as_ref().is_some_and(|path| {
                std::fs::canonicalize(inventory_root.join(path))
                    .is_ok_and(|path| path == transcript)
            })
    });
    if !recorded {
        return Err(format!(
            "evidence record {id} must reference a recorded public-driver transcript"
        ));
    }
    let fixture =
        crate::load_public_driver_fixture(&transcript).map_err(|error| error.to_string())?;
    let provider = non_empty(record, "provider")?;
    let version = non_empty(record, "provider_version")?;
    let platform = non_empty(record, "platform")?;
    let transport = non_empty(record, "driver_transport")?;
    let metadata = &fixture.metadata;
    require_metadata(metadata, "vendor", &provider, id)?;
    require_metadata(metadata, "cliVersion", &version, id)?;
    let platform_value = metadata_string(metadata, "platform", id)?;
    if !platform_value.starts_with(&format!("{platform}-")) {
        return Err(format!(
            "evidence record {id} platform does not match driver transcript"
        ));
    }
    require_metadata(metadata, "transport", &transport, id)
}

fn record_path(record: &Mapping, id: &str, root: &Path) -> Result<PathBuf, String> {
    let relative = PathBuf::from(non_empty(record, "driver_transcript")?);
    if relative
        .extension()
        .is_none_or(|extension| extension != "jsonl")
        || relative.is_absolute()
        || relative.components().any(|part| part.as_os_str() == "..")
    {
        return Err(format!(
            "evidence record {id} driver_transcript must be repository-relative"
        ));
    }
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("invalid coverage-manifest directory: {error}"))?;
    let path = std::fs::canonicalize(root.join(relative)).map_err(|error| {
        format!("evidence record {id} could not resolve driver_transcript: {error}")
    })?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(format!(
            "evidence record {id} driver_transcript must be inside fixtures"
        ));
    }
    Ok(path)
}

fn require_metadata(
    metadata: &std::collections::BTreeMap<String, serde_json::Value>,
    field: &str,
    expected: &str,
    id: &str,
) -> Result<(), String> {
    (metadata_string(metadata, field, id)? == expected)
        .then_some(())
        .ok_or_else(|| format!("evidence record {id} {field} does not match driver transcript"))
}

fn metadata_string(
    metadata: &std::collections::BTreeMap<String, serde_json::Value>,
    field: &str,
    id: &str,
) -> Result<String, String> {
    metadata
        .get(field)
        .and_then(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_i64().map(|value| value.to_string()))
        })
        .ok_or_else(|| format!("evidence record {id} driver transcript lacks {field}"))
}
