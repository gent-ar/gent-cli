use std::path::Path;

use serde_yaml::{Mapping, Value};

mod evidence_records;

/// Validates declared coverage and recorded evidence without inventing any evidence.
///
/// # Errors
/// Returns a useful validation error when a manifest is incomplete, inconsistent, or
/// attempts authority transfer without the full verified evidence set.
pub fn validate_evidence_manifest(
    manifest_path: &Path,
    authority_transfer: bool,
) -> Result<(), String> {
    let contents = std::fs::read_to_string(manifest_path)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let manifest: Value =
        serde_yaml::from_str(&contents).map_err(|error| format!("invalid YAML: {error}"))?;
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    validate(&manifest, manifest_dir, authority_transfer)
}

fn validate(manifest: &Value, manifest_dir: &Path, authority_transfer: bool) -> Result<(), String> {
    let root = mapping(manifest, "root")?;
    require_scalar(root, "schema_version", "3")?;
    let dimensions = mapping(required(root, "dimensions")?, "dimensions")?;
    for (name, expected) in [
        (
            "providers",
            ["claude", "codex", "claurst", "copilot"].as_slice(),
        ),
        ("transports", ["local_ipc", "paired_mux"].as_slice()),
        ("platforms", ["macos", "linux", "windows"].as_slice()),
    ] {
        let values = strings(required(dimensions, name)?, name)?;
        for value in expected {
            if !values.contains(&value.to_string()) {
                return Err(format!("dimensions.{name} is missing {value}"));
            }
        }
    }
    let provider_implementation = mapping(
        required(root, "provider_implementation")?,
        "provider_implementation",
    )?;
    require_scalar(provider_implementation, "copilot", "unsupported")?;
    let features = mapping(required(root, "features")?, "features")?;
    validate_features(features)?;
    let records = root.get(Value::String("evidence_records".into()));
    if let Some(records) = records {
        evidence_records::validate_records(
            records,
            features,
            dimensions,
            manifest_dir,
            authority_transfer,
        )
    } else if authority_transfer {
        Err("authority transfer is blocked: phase-0 evidence_records are absent; real provider evidence must be recorded, never fabricated".into())
    } else {
        Ok(())
    }
}

fn validate_features(features: &Mapping) -> Result<(), String> {
    if features.is_empty() {
        return Err("features must not be empty".into());
    }
    let valid_states = [
        "supported",
        "emulated",
        "unsupported",
        "not_applicable",
        "temporarily_unavailable",
    ];
    for (name, feature) in features {
        let name = name
            .as_str()
            .ok_or_else(|| "feature key must be a string".to_owned())?;
        let feature = mapping(feature, name)?;
        let state = scalar(
            required(feature, "state")?,
            &format!("features.{name}.state"),
        )?;
        if !valid_states.contains(&state.as_str()) {
            return Err(format!("features.{name}.state has unknown value {state}"));
        }
        require_scalar(feature, "evidence", "required")?;
        let owner = scalar(
            required(feature, "legacy_owner")?,
            &format!("features.{name}.legacy_owner"),
        )?;
        if state == "not_applicable"
            && (owner != "none" || !feature.contains_key(Value::String("rationale".into())))
        {
            return Err(format!(
                "features.{name} requires legacy_owner: none and a rationale"
            ));
        }
        if state == "temporarily_unavailable"
            && (!feature.contains_key(Value::String("exception_expiry".into())) || owner == "none")
        {
            return Err(format!(
                "features.{name} requires exception_expiry and a non-empty owner"
            ));
        }
    }
    Ok(())
}

pub(super) fn non_empty(mapping: &Mapping, key: &str) -> Result<String, String> {
    let value = scalar(required(mapping, key)?, key)?;
    (!value.trim().is_empty())
        .then_some(value)
        .ok_or_else(|| format!("{key} must not be empty"))
}

pub(super) fn member(mapping: &Mapping, key: &str, values: &[String]) -> Result<String, String> {
    let value = non_empty(mapping, key)?;
    values
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| format!("{key} has unknown dimension value"))
}

pub(super) fn mapping<'a>(value: &'a Value, name: &str) -> Result<&'a Mapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| format!("{name} must be a mapping"))
}

pub(super) fn required<'a>(mapping: &'a Mapping, key: &str) -> Result<&'a Value, String> {
    mapping
        .get(Value::String(key.into()))
        .ok_or_else(|| format!("missing required key {key}"))
}

pub(super) fn scalar(value: &Value, name: &str) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .ok_or_else(|| format!("{name} must be a scalar"))
}

fn require_scalar(mapping: &Mapping, key: &str, expected: &str) -> Result<(), String> {
    let actual = scalar(required(mapping, key)?, key)?;
    (actual == expected)
        .then_some(())
        .ok_or_else(|| format!("{key} must be {expected}, got {actual}"))
}

pub(super) fn strings(value: &Value, name: &str) -> Result<Vec<String>, String> {
    value
        .as_sequence()
        .ok_or_else(|| format!("{name} must be a sequence"))?
        .iter()
        .map(|value| scalar(value, name))
        .collect()
}

#[cfg(test)]
#[path = "evidence_manifest_tests.rs"]
mod tests;
