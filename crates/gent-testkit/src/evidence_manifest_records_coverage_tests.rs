use std::collections::BTreeSet;

use serde_yaml::{Mapping, Value};

use super::{iso_date_days, validate_coverage, validate_exception_expiry};

fn features() -> Mapping {
    serde_yaml::from_str::<Value>("example: { state: supported }")
        .unwrap()
        .as_mapping()
        .unwrap()
        .clone()
}

#[test]
fn coverage_requires_and_accepts_each_declared_dimension_cell() {
    let providers = vec!["claude".into()];
    let platforms = vec!["macos".into()];
    let transports = vec!["local_ipc".into()];
    let mut covered = BTreeSet::new();
    assert!(
        validate_coverage(&features(), &providers, &platforms, &transports, &covered,).is_err()
    );
    covered.insert((
        "example".into(),
        "claude".into(),
        "macos".into(),
        "local_ipc".into(),
    ));
    assert!(validate_coverage(&features(), &providers, &platforms, &transports, &covered,).is_ok());
}

#[test]
fn expiry_parser_rejects_impossible_dates_before_clock_comparison() {
    assert!(iso_date_days("2024-02-29").is_some());
    for value in [
        "2023-02-29",
        "1969-12-31",
        "2024-13-01",
        "2024-01-32",
        "2024-1-01",
    ] {
        assert_eq!(iso_date_days(value), None);
    }
    assert!(validate_exception_expiry("2999-01-01").is_ok());
    assert!(validate_exception_expiry("1970-01-01").is_err());
}
