use std::path::PathBuf;

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
    validate(&manifest, args.authority_transfer)?;
    println!(
        "coverage manifest is structurally valid: {}",
        args.manifest.display()
    );
    Ok(())
}

fn validate(manifest: &Value, authority_transfer: bool) -> Result<(), String> {
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
    if authority_transfer && !root.contains_key(Value::String("evidence_records".into())) {
        return Err("authority transfer is blocked: phase-0 evidence_records are absent; real provider evidence must be recorded, never fabricated".into());
    }
    Ok(())
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

    #[test]
    fn rejects_unknown_feature_state() {
        let fixture = r"
schema_version: 3
dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }
provider_implementation: { copilot: unsupported }
features: { example: { state: invented, evidence: required, legacy_owner: owner } }
";
        let value: Value = serde_yaml::from_str(fixture).unwrap();
        assert!(validate(&value, false).is_err());
    }
}
