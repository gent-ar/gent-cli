use std::{collections::BTreeSet, path::Path};

use serde_yaml::{Mapping, Value};

use super::{mapping, member, non_empty, required, scalar, strings};

pub(super) fn validate_records(
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
        validate_record(record, &id, &state, manifest_dir, authority_transfer)?;
        covered.insert((feature, provider, platform, transport));
    }
    if authority_transfer {
        validate_coverage(features, &providers, &platforms, &transports, &covered)?;
    }
    Ok(())
}

fn validate_record(
    record: &Mapping,
    id: &str,
    state: &str,
    manifest_dir: &Path,
    authority_transfer: bool,
) -> Result<(), String> {
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
    validate_paths(record, id, manifest_dir, status == "passed")?;
    let artifact = non_empty(record, "ci_artifact")?;
    if !artifact.starts_with("signed:") || artifact.len() == "signed:".len() {
        return Err(format!(
            "evidence record {id} ci_artifact must be a signed artifact id"
        ));
    }
    if non_empty(record, "provider")? == "claurst"
        && !non_empty(record, "private_evidence")?.starts_with("signed:")
    {
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
    Ok(())
}

fn validate_coverage(
    features: &Mapping,
    providers: &[String],
    platforms: &[String],
    transports: &[String],
    covered: &BTreeSet<(String, String, String, String)>,
) -> Result<(), String> {
    for feature in features.keys().filter_map(Value::as_str) {
        for provider in providers {
            for platform in platforms {
                for transport in transports {
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
