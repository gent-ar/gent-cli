use std::{
    collections::BTreeSet,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_yaml::{Mapping, Value};

use super::{mapping, member, non_empty, required, scalar, strings};

pub(super) fn validate_records(
    value: &Value,
    features: &Mapping,
    dimensions: &Mapping,
    provider_implementation: &Mapping,
    required_evidence: &[String],
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
        let expected_implementation = non_empty(provider_implementation, &provider)?;
        validate_record(
            record,
            &id,
            &state,
            &expected_implementation,
            required_evidence,
            manifest_dir,
            authority_transfer,
        )?;
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
    expected_implementation: &str,
    required_evidence: &[String],
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
    validate_paths(record, id, required_evidence, manifest_dir)?;
    if non_empty(record, "provider_implementation")? != expected_implementation {
        return Err(format!(
            "evidence record {id} has the wrong provider implementation"
        ));
    }
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
        let expiry = non_empty(record, "exception_expiry")?;
        validate_exception_expiry(&expiry)?;
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

fn validate_exception_expiry(value: &str) -> Result<(), String> {
    let expiry_days = iso_date_days(value)
        .ok_or_else(|| "exception_expiry must use the YYYY-MM-DD format".to_owned())?;
    let now_days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system time is before the Unix epoch".to_owned())?
        .as_secs()
        / 86_400;
    if expiry_days < now_days {
        return Err(format!("exception_expiry {value} has expired"));
    }
    Ok(())
}

fn iso_date_days(value: &str) -> Option<u64> {
    let mut parts = value.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let year = year.parse::<u64>().ok()?;
    let month = month.parse::<u64>().ok()?;
    let day = day.parse::<u64>().ok()?;
    if year < 1970 || !(1..=12).contains(&month) || day == 0 || day > month_days(year, month) {
        return None;
    }
    let prior_years = (1970..year)
        .map(|candidate| if leap_year(candidate) { 366 } else { 365 })
        .sum::<u64>();
    let prior_months = (1..month)
        .map(|candidate| month_days(year, candidate))
        .sum::<u64>();
    Some(prior_years + prior_months + day - 1)
}

const fn leap_year(year: u64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn month_days(year: u64, month: u64) -> u64 {
    match month {
        2 if leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
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
    required_evidence: &[String],
    manifest_dir: &Path,
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
    for required_kind in required_evidence {
        if !paths.contains_key(Value::String(required_kind.clone())) {
            return Err(format!(
                "evidence record {id} is missing {required_kind} evidence"
            ));
        }
    }
    Ok(())
}
