//! Structural provenance requirements for live public-driver captures.

use std::collections::BTreeMap;

use serde_json::Value;

pub(super) fn validate_live_provenance(
    metadata: &BTreeMap<String, Value>,
    cell: &str,
    absent: bool,
    errors: &mut Vec<String>,
) {
    require(metadata, "captureOrigin", cell, errors, |value| {
        value == "live_cli"
    });
    require(metadata, "executablePath", cell, errors, |value| {
        std::path::Path::new(value).is_absolute()
    });
    require(metadata, "executableDigest", cell, errors, sha256_digest);
    require(metadata, "platform", cell, errors, supported_platform);
    require(metadata, "transport", cell, errors, supported_transport);
    require(metadata, "captureRunId", cell, errors, |value| {
        !value.is_empty()
    });
    require(metadata, "attestationDigest", cell, errors, sha256_digest);
    require(metadata, "capturedAt", cell, errors, rfc3339_timestamp);
    if absent {
        require(metadata, "observedCommand", cell, errors, |value| {
            !value.is_empty()
        });
        require(metadata, "observedResult", cell, errors, |value| {
            !value.is_empty()
        });
    }
}

fn require(
    metadata: &BTreeMap<String, Value>,
    field: &str,
    cell: &str,
    errors: &mut Vec<String>,
    predicate: impl FnOnce(&str) -> bool,
) {
    if metadata
        .get(field)
        .and_then(Value::as_str)
        .is_none_or(|value| !predicate(value))
    {
        errors.push(format!("invalid or missing {field} for {cell}"));
    }
}

fn sha256_digest(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return false;
    };
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn supported_platform(value: &str) -> bool {
    ["macos-", "linux-", "windows-"]
        .into_iter()
        .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len())
}

fn supported_transport(value: &str) -> bool {
    matches!(value, "stream_json" | "json_rpc" | "stdio")
}

fn rfc3339_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 20
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && (value.ends_with('Z') || value.rfind('+').is_some())
}
