use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use clap::Parser;
use serde_yaml::{Mapping, Value};

#[derive(Debug, Parser)]
#[command(
    about = "Validates the Gent coverage-manifest evidence graph without fabricating evidence"
)]
struct Args {
    #[arg(default_value = "fixtures/coverage-manifest.yml")]
    manifest: PathBuf,
    /// Enforce the deliberately stricter, post-phase-0 authority-transfer evidence requirements.
    #[arg(long)]
    authority_transfer: bool,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    let contents = std::fs::read_to_string(&args.manifest)
        .map_err(|error| format!("{}: {error}", args.manifest.display()))?;
    let manifest: Value =
        serde_yaml::from_str(&contents).map_err(|error| format!("invalid YAML: {error}"))?;
    let manifest_dir = args.manifest.parent().unwrap_or_else(|| Path::new("."));
    validate(&manifest, manifest_dir, args.authority_transfer)?;
    println!(
        "coverage manifest is structurally valid: {}",
        args.manifest.display()
    );
    Ok(())
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
    let records = root.get(Value::String("evidence_records".into()));
    if let Some(records) = records {
        validate_records(
            records,
            features,
            dimensions,
            manifest_dir,
            authority_transfer,
        )?;
    } else if authority_transfer {
        return Err("authority transfer is blocked: phase-0 evidence_records are absent; real provider evidence must be recorded, never fabricated".into());
    }
    Ok(())
}

fn validate_records(
    value: &Value,
    features: &Mapping,
    dimensions: &Mapping,
    manifest_dir: &Path,
    authority_transfer: bool,
) -> Result<(), String> {
    let records = value
        .as_sequence()
        .ok_or_else(|| "evidence_records must be a sequence".to_owned())?;
    if records.is_empty() {
        return Err(
            "evidence_records must not be empty; omit it until real evidence exists".into(),
        );
    }
    let providers = strings(required(dimensions, "providers")?, "providers")?;
    let platforms = strings(required(dimensions, "platforms")?, "platforms")?;
    let transports = strings(required(dimensions, "transports")?, "transports")?;
    let mut ids = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for (index, record) in records.iter().enumerate() {
        let record = mapping(record, &format!("evidence_records[{index}]"))?;
        let id = non_empty(record, "id")?;
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate evidence record id {id}"));
        }
        let feature = non_empty(record, "feature")?;
        let feature_config = mapping(required(features, &feature)?, &format!("feature {feature}"))?;
        let state = non_empty(record, "state")?;
        if state != scalar(required(feature_config, "state")?, "feature state")? {
            return Err(format!(
                "evidence record {id} state does not match feature {feature}"
            ));
        }
        let provider = member(record, "provider", &providers)?;
        let platform = member(record, "platform", &platforms)?;
        let transport = member(record, "transport", &transports)?;
        let version = non_empty(record, "provider_version")?;
        if version == "not_applicable" && state != "not_applicable" {
            return Err(format!(
                "evidence record {id} has no provider version for an applicable state"
            ));
        }
        let status = non_empty(record, "status")?;
        if !["passed", "recorded_absent", "failed"].contains(&status.as_str()) {
            return Err(format!("evidence record {id} has unknown status {status}"));
        }
        validate_paths(record, &id, manifest_dir, status == "passed")?;
        let artifact = non_empty(record, "ci_artifact")?;
        if !artifact.starts_with("signed:") || artifact.len() == "signed:".len() {
            return Err(format!(
                "evidence record {id} ci_artifact must be a signed artifact id"
            ));
        }
        if provider == "claurst" && !non_empty(record, "private_evidence")?.starts_with("signed:") {
            return Err(format!(
                "evidence record {id} requires signed private Claurst evidence"
            ));
        }
        if state == "temporarily_unavailable" {
            let _ = non_empty(record, "exception_expiry")?;
        }
        if state == "not_applicable" {
            let _ = non_empty(record, "rationale")?;
        }
        if authority_transfer {
            if status != "passed" {
                return Err(format!(
                    "authority transfer is blocked: evidence record {id} is {status}"
                ));
            }
            let _ = non_empty(record, "legacy_path")?;
            let _ = non_empty(record, "removal_approval")?;
            let _ = non_empty(record, "removal_release")?;
        }
        covered.insert((feature, provider, platform, transport));
    }
    if authority_transfer {
        for feature in features.keys().filter_map(Value::as_str) {
            for provider in &providers {
                for platform in &platforms {
                    for transport in &transports {
                        if !covered.contains(&(
                            feature.to_owned(),
                            provider.clone(),
                            platform.clone(),
                            transport.clone(),
                        )) {
                            return Err(format!(
                                "authority transfer is blocked: missing evidence for {feature}/{provider}/{platform}/{transport}"
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_paths(
    record: &Mapping,
    id: &str,
    manifest_dir: &Path,
    all_required: bool,
) -> Result<(), String> {
    let paths = mapping(required(record, "evidence_paths")?, "evidence_paths")?;
    if paths.is_empty() {
        return Err(format!("evidence record {id} has no evidence paths"));
    }
    for (kind, path) in paths {
        let kind = scalar(kind, "evidence path kind")?;
        let path = scalar(path, "evidence path")?;
        let relative = Path::new(&path);
        if relative.is_absolute()
            || relative.components().any(|part| part.as_os_str() == "..")
            || !manifest_dir.join(relative).is_file()
        {
            return Err(format!(
                "evidence record {id} has missing or non-repository path {path}"
            ));
        }
        if kind.is_empty() {
            return Err(format!("evidence record {id} has an empty evidence kind"));
        }
    }
    if all_required && !paths.contains_key(Value::String("cutover".into())) {
        return Err(format!(
            "evidence record {id} is passed without a cutover evidence path"
        ));
    }
    Ok(())
}

fn non_empty(mapping: &Mapping, key: &str) -> Result<String, String> {
    let value = scalar(required(mapping, key)?, key)?;
    (!value.trim().is_empty())
        .then_some(value)
        .ok_or_else(|| format!("{key} must not be empty"))
}

fn member(mapping: &Mapping, key: &str, values: &[String]) -> Result<String, String> {
    let value = non_empty(mapping, key)?;
    values
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| format!("{key} has unknown dimension value"))
}

fn mapping<'a>(value: &'a Value, name: &str) -> Result<&'a Mapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| format!("{name} must be a mapping"))
}

fn required<'a>(mapping: &'a Mapping, key: &str) -> Result<&'a Value, String> {
    mapping
        .get(Value::String(key.into()))
        .ok_or_else(|| format!("missing required key {key}"))
}

fn scalar(value: &Value, name: &str) -> Result<String, String> {
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

fn strings(value: &Value, name: &str) -> Result<Vec<String>, String> {
    value
        .as_sequence()
        .ok_or_else(|| format!("{name} must be a sequence"))?
        .iter()
        .map(|value| scalar(value, name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::validate;
    use serde_yaml::Value;
    use std::path::Path;

    #[test]
    fn rejects_unknown_feature_state() {
        let fixture = r"
schema_version: 3
dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }
provider_implementation: { copilot: unsupported }
features: { example: { state: invented, evidence: required, legacy_owner: owner } }
";
        let value: Value = serde_yaml::from_str(fixture).unwrap();
        assert!(validate(&value, Path::new("."), false).is_err());
    }

    #[test]
    fn baseline_is_valid_but_cannot_transfer_authority_without_real_evidence() {
        let value: Value =
            serde_yaml::from_str(include_str!("../../../../fixtures/coverage-manifest.yml"))
                .unwrap();
        assert!(validate(&value, Path::new("."), false).is_ok());
        assert!(
            validate(&value, Path::new("."), true)
                .unwrap_err()
                .contains("evidence_records are absent")
        );
    }
}
