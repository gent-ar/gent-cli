use crate::{windows_helper_inspection::inspect, windows_sandbox_runtime::PinnedSandboxHelper};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema: u8,
    nonce: String,
    path: PathBuf,
    sha256: String,
    protected: bool,
}

pub(super) fn verify(
    helper: &PinnedSandboxHelper,
    expected_path: &Path,
    digest: &str,
) -> Result<(), String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let report: Report = inspect(
        helper,
        "inspect-globalization-data",
        std::slice::from_ref(&nonce),
    )?;
    validate(&report, &nonce, expected_path, digest)
}

fn validate(
    report: &Report,
    nonce: &str,
    expected_path: &Path,
    digest: &str,
) -> Result<(), String> {
    if report.schema != 1
        || report.nonce != nonce
        || !report.protected
        || !report
            .path
            .as_os_str()
            .eq_ignore_ascii_case(expected_path.as_os_str())
        || report.sha256 != digest
    {
        return Err("OS globalization data differs from reviewed protected authority".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globalization_data_binds_fresh_protected_exact_path_and_hash() {
        let path =
            gent_testkit::host_absolute_path("/Windows/Globalization/Sorting/sortdefault.nls");
        let nonce = uuid::Uuid::new_v4().to_string();
        let mut report = Report {
            schema: 1,
            nonce: nonce.clone(),
            path: path.clone(),
            sha256: "a".repeat(64),
            protected: true,
        };
        assert!(validate(&report, &nonce, &path, &"a".repeat(64)).is_ok());
        report.protected = false;
        assert!(validate(&report, &nonce, &path, &"a".repeat(64)).is_err());
        report.protected = true;
        assert!(validate(&report, "old", &path, &"a".repeat(64)).is_err());
        assert!(validate(&report, &nonce, &path, &"b".repeat(64)).is_err());
        report.path = path.with_file_name("other.nls");
        assert!(validate(&report, &nonce, &path, &"a".repeat(64)).is_err());
    }
}
