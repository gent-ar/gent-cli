//! Declarative capture-readiness validation for phase-0 public transcripts.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    PUBLIC_PROVIDERS, PublicDriverFixture, REQUIRED_SCENARIOS, transcript_fixture::load_fixture,
    transcript_provenance::validate_live_provenance,
};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u8,
    vendors: Vec<String>,
    scenarios: Vec<String>,
    cells: Vec<Cell>,
}

#[derive(Debug, Deserialize)]
struct Cell {
    vendor: String,
    scenario: String,
    state: CellState,
    path: Option<PathBuf>,
    rationale: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CellState {
    CaptureRequired,
    Recorded,
    RecordedAbsent,
    Synthetic,
}

#[derive(Clone, Copy)]
enum FixtureKind {
    Recorded,
    RecordedAbsent,
    Synthetic,
}

/// Validates a public-driver capture inventory without treating it as evidence.
///
/// The default mode proves each required matrix cell is explicitly inventoried.
/// `require_live` additionally rejects uncaptured and synthetic cells, and
/// validates every declared recording's live-capture provenance and hygiene.
///
/// # Errors
/// Returns all structural, provenance, and fixture errors in one stable report.
pub fn validate_public_driver_manifest(path: &Path, require_live: bool) -> Result<(), String> {
    let contents =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let manifest: Manifest =
        serde_yaml::from_str(&contents).map_err(|error| format!("invalid YAML: {error}"))?;
    let mut errors = Vec::new();
    if manifest.schema_version != 1 {
        errors.push("schema_version must be 1".into());
    }
    validate_dimensions(&manifest, &mut errors);
    validate_cells(
        path.parent().unwrap_or_else(|| Path::new(".")),
        &manifest,
        require_live,
        &mut errors,
    );
    errors
        .is_empty()
        .then_some(())
        .ok_or_else(|| errors.join("\n"))
}

fn validate_dimensions(manifest: &Manifest, errors: &mut Vec<String>) {
    let expected_vendors = PUBLIC_PROVIDERS
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let expected_scenarios = REQUIRED_SCENARIOS
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let vendors = manifest.vendors.iter().cloned().collect::<BTreeSet<_>>();
    if vendors.len() != manifest.vendors.len() || vendors != expected_vendors {
        errors.push("vendors must declare exactly claude and codex".into());
    }
    let scenarios = manifest.scenarios.iter().cloned().collect::<BTreeSet<_>>();
    if scenarios.len() != manifest.scenarios.len() || scenarios != expected_scenarios {
        errors.push("scenarios must declare the complete public-driver scenario matrix".into());
    }
}

fn validate_cells(root: &Path, manifest: &Manifest, require_live: bool, errors: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for cell in &manifest.cells {
        let key = (cell.vendor.clone(), cell.scenario.clone());
        if !seen.insert(key.clone()) {
            errors.push(format!(
                "duplicate transcript matrix cell {}/{}",
                key.0, key.1
            ));
            continue;
        }
        if !PUBLIC_PROVIDERS.contains(&cell.vendor.as_str())
            || !REQUIRED_SCENARIOS.contains(&cell.scenario.as_str())
        {
            errors.push(format!(
                "unknown transcript matrix cell {}/{}",
                cell.vendor, cell.scenario
            ));
            continue;
        }
        validate_cell(root, cell, require_live, errors);
    }
    for vendor in PUBLIC_PROVIDERS {
        for scenario in REQUIRED_SCENARIOS {
            if !seen.contains(&(vendor.to_owned(), scenario.to_owned())) {
                errors.push(format!(
                    "missing transcript matrix cell {vendor}/{scenario}"
                ));
            }
        }
    }
}

fn validate_cell(root: &Path, cell: &Cell, require_live: bool, errors: &mut Vec<String>) {
    match cell.state {
        CellState::CaptureRequired => {
            if cell.path.is_some() || cell.rationale.is_some() {
                errors.push(format!(
                    "capture_required cell {}/{} must not claim a fixture",
                    cell.vendor, cell.scenario
                ));
            }
            if require_live {
                errors.push(format!(
                    "live transcript capture is still required for {}/{}",
                    cell.vendor, cell.scenario
                ));
            }
        }
        CellState::Synthetic => {
            require_non_empty(cell.rationale.as_deref(), "rationale", cell, errors);
            if require_live {
                errors.push(format!(
                    "synthetic transcript cannot satisfy live evidence for {}/{}",
                    cell.vendor, cell.scenario
                ));
            }
            validate_fixture(root, cell, FixtureKind::Synthetic, errors);
        }
        CellState::Recorded => validate_fixture(root, cell, FixtureKind::Recorded, errors),
        CellState::RecordedAbsent => {
            if require_live {
                errors.push(format!(
                    "recorded absence cannot satisfy live provider evidence for {}/{}",
                    cell.vendor, cell.scenario
                ));
            }
            validate_fixture(root, cell, FixtureKind::RecordedAbsent, errors);
        }
    }
}

fn require_non_empty(value: Option<&str>, field: &str, cell: &Cell, errors: &mut Vec<String>) {
    if value.is_none_or(str::is_empty) {
        errors.push(format!(
            "{field} is required for {}/{}",
            cell.vendor, cell.scenario
        ));
    }
}

fn validate_fixture(root: &Path, cell: &Cell, kind: FixtureKind, errors: &mut Vec<String>) {
    let fixture = match load_fixture(root, cell.path.as_deref(), &cell.vendor, &cell.scenario) {
        Ok(fixture) => fixture,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    validate_identity(&fixture, cell, errors);
    validate_kind(&fixture, cell, kind, errors);
}

fn validate_identity(fixture: &PublicDriverFixture, cell: &Cell, errors: &mut Vec<String>) {
    let metadata = &fixture.metadata;
    if metadata.get("vendor").and_then(serde_json::Value::as_str) != Some(cell.vendor.as_str()) {
        errors.push(format!(
            "fixture vendor does not match {}/{}",
            cell.vendor, cell.scenario
        ));
    }
    if metadata.get("scenario").and_then(serde_json::Value::as_str) != Some(cell.scenario.as_str())
    {
        errors.push(format!(
            "fixture scenario does not match {}/{}",
            cell.vendor, cell.scenario
        ));
    }
}

fn validate_kind(
    fixture: &PublicDriverFixture,
    cell: &Cell,
    kind: FixtureKind,
    errors: &mut Vec<String>,
) {
    let metadata = &fixture.metadata;
    let expected_status = match kind {
        FixtureKind::Recorded => "recorded",
        FixtureKind::RecordedAbsent => "recorded_absent",
        FixtureKind::Synthetic => "synthetic",
    };
    if metadata.get("status").and_then(serde_json::Value::as_str) != Some(expected_status) {
        errors.push(format!(
            "fixture status must be {expected_status} for {}/{}",
            cell.vendor, cell.scenario
        ));
    }
    if matches!(kind, FixtureKind::Recorded | FixtureKind::Synthetic) && fixture.frames.is_empty() {
        errors.push(format!(
            "{expected_status} fixture has no frames for {}/{}",
            cell.vendor, cell.scenario
        ));
    }
    if matches!(kind, FixtureKind::Recorded) {
        validate_live_provenance(
            metadata,
            &format!("{}/{}", cell.vendor, cell.scenario),
            false,
            errors,
        );
    }
    if matches!(kind, FixtureKind::Synthetic)
        && metadata
            .get("captureOrigin")
            .and_then(serde_json::Value::as_str)
            != Some("synthetic")
    {
        errors.push(format!(
            "synthetic fixture is not marked synthetic for {}/{}",
            cell.vendor, cell.scenario
        ));
    }
    if matches!(kind, FixtureKind::RecordedAbsent) {
        if !fixture.frames.is_empty() {
            errors.push(format!(
                "recorded-absent fixture has frames for {}/{}",
                cell.vendor, cell.scenario
            ));
        } else if metadata
            .get("absenceReason")
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
        {
            errors.push(format!(
                "recorded-absent fixture needs absenceReason for {}/{}",
                cell.vendor, cell.scenario
            ));
        }
        validate_live_provenance(
            metadata,
            &format!("{}/{}", cell.vendor, cell.scenario),
            true,
            errors,
        );
    }
}
