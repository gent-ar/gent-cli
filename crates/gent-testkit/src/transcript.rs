//! Loader and hygiene gate for real, redacted public-driver recordings.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const REQUIRED_META: [&str; 9] = [
    "vendor",
    "scenario",
    "capturedAt",
    "cliVersion",
    "adapterSpecVersion",
    "appVersion",
    "prompt",
    "repo",
    "notes",
];

#[derive(Clone, Debug, PartialEq)]
pub struct PublicDriverFixture {
    pub path: PathBuf,
    pub metadata: BTreeMap<String, Value>,
    pub frames: Vec<PublicDriverFrame>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PublicDriverFrame {
    pub input: Value,
    pub expectation: Option<String>,
    pub expectation_fields: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum TranscriptError {
    #[error("could not read transcript {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("transcript {path} line {line} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    #[error("transcript {path} is empty")]
    Empty { path: PathBuf },
    #[error("transcript {path} line {line}: {reason}")]
    Invalid {
        path: PathBuf,
        line: usize,
        reason: String,
    },
    #[error("transcript {path} line {line} contains possible secret ({rule}) at {json_path}")]
    PossibleSecret {
        path: PathBuf,
        line: usize,
        json_path: String,
        rule: &'static str,
    },
}

/// Loads one recording, verifies its provenance, and rejects unredacted secrets.
///
/// # Errors
/// Returns an error for unreadable or malformed recordings, missing provenance,
/// unsupported providers, or a possible unredacted secret.
pub fn load_public_driver_fixture(
    path: impl AsRef<Path>,
) -> Result<PublicDriverFixture, TranscriptError> {
    let path = path.as_ref().to_path_buf();
    let content = fs::read_to_string(&path).map_err(|source| TranscriptError::Read {
        path: path.clone(),
        source,
    })?;
    let mut lines = content
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    let Some((_, header)) = lines.next() else {
        return Err(TranscriptError::Empty { path });
    };
    let metadata = parse_metadata(&path, header)?;
    let mut frames = Vec::new();
    for (index, line) in lines {
        let line_number = index + 1;
        let value = parse_line(&path, line_number, line)?;
        validate_redaction(&path, line_number, &value, "$")?;
        frames.push(parse_frame(&path, line_number, &value)?);
    }
    Ok(PublicDriverFixture {
        path,
        metadata,
        frames,
    })
}

/// Loads all `.jsonl` files directly under a provider fixture directory in path order.
///
/// # Errors
/// Returns an error when the directory cannot be read or one recording fails
/// [`load_public_driver_fixture`]'s provenance and redaction checks.
pub fn load_public_driver_fixtures(
    directory: impl AsRef<Path>,
) -> Result<Vec<PublicDriverFixture>, TranscriptError> {
    let directory = directory.as_ref();
    let mut paths = fs::read_dir(directory)
        .map_err(|source| TranscriptError::Read {
            path: directory.to_path_buf(),
            source,
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| TranscriptError::Read {
            path: directory.to_path_buf(),
            source,
        })?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    });
    paths.sort();
    paths.into_iter().map(load_public_driver_fixture).collect()
}

fn parse_metadata(path: &Path, line: &str) -> Result<BTreeMap<String, Value>, TranscriptError> {
    let value = parse_line(path, 1, line)?;
    validate_redaction(path, 1, &value, "$")?;
    let Some(metadata) = value.get("meta").and_then(Value::as_object) else {
        return invalid(path, 1, "first line must contain a meta object");
    };
    if value.as_object().is_none_or(|object| object.len() != 1) {
        return invalid(path, 1, "meta header cannot contain frame fields");
    }
    for required in REQUIRED_META {
        if !valid_metadata_value(metadata.get(required), required) {
            return invalid(
                path,
                1,
                &format!("meta.{required} must be a non-empty string"),
            );
        }
    }
    let vendor = metadata
        .get("vendor")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(vendor, "claude" | "codex") {
        return invalid(
            path,
            1,
            "meta.vendor must be claude or codex for a public driver",
        );
    }
    Ok(metadata.clone().into_iter().collect())
}

fn valid_metadata_value(value: Option<&Value>, field: &str) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
        || (field == "adapterSpecVersion" && value.and_then(Value::as_i64).is_some())
}

fn parse_frame(
    path: &Path,
    line: usize,
    value: &Value,
) -> Result<PublicDriverFrame, TranscriptError> {
    let Some(object) = value.as_object() else {
        return invalid(path, line, "frame must be a JSON object");
    };
    if object.contains_key("meta") {
        return invalid(path, line, "meta is allowed only on the first line");
    }
    let Some(input) = object.get("in") else {
        return invalid(path, line, "frame is missing in");
    };
    let Some(expectation) = object.get("expect") else {
        return invalid(path, line, "frame is missing expect");
    };
    if !expectation.is_null() && !expectation.is_string() {
        return invalid(path, line, "expect must be a string or null");
    }
    let expectation_fields = object.get("expectFields").cloned();
    if expectation_fields
        .as_ref()
        .is_some_and(|fields| !fields.is_object())
    {
        return invalid(path, line, "expectFields must be an object when present");
    }
    Ok(PublicDriverFrame {
        input: input.clone(),
        expectation: expectation.as_str().map(str::to_owned),
        expectation_fields,
    })
}

fn parse_line(path: &Path, line: usize, source: &str) -> Result<Value, TranscriptError> {
    serde_json::from_str(source).map_err(|source| TranscriptError::Json {
        path: path.to_path_buf(),
        line,
        source,
    })
}

fn invalid<T>(path: &Path, line: usize, reason: &str) -> Result<T, TranscriptError> {
    Err(TranscriptError::Invalid {
        path: path.to_path_buf(),
        line,
        reason: reason.into(),
    })
}

fn validate_redaction(
    path: &Path,
    line: usize,
    value: &Value,
    json_path: &str,
) -> Result<(), TranscriptError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{json_path}.{key}");
                if is_sensitive_key(key) {
                    return secret(path, line, &child_path, "sensitive field name");
                }
                validate_redaction(path, line, child, &child_path)?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                validate_redaction(path, line, child, &format!("{json_path}[{index}]"))?;
            }
        }
        Value::String(text) if secret_rule(text).is_some() => {
            return secret(path, line, json_path, secret_rule(text).unwrap_or_default());
        }
        _ => {}
    }
    Ok(())
}

fn secret(
    path: &Path,
    line: usize,
    json_path: &str,
    rule: &'static str,
) -> Result<(), TranscriptError> {
    Err(TranscriptError::PossibleSecret {
        path: path.to_path_buf(),
        line,
        json_path: json_path.into(),
        rule,
    })
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
    matches!(
        normalized.as_str(),
        "authorization" | "apikey" | "password" | "secret" | "token"
    )
}

fn secret_rule(text: &str) -> Option<&'static str> {
    let text = text.to_ascii_lowercase();
    [
        ("sk-", "OpenAI or Anthropic key prefix"),
        ("ghp_", "GitHub personal token prefix"),
        ("github_pat_", "GitHub fine-grained token prefix"),
        ("bearer ", "bearer credential marker"),
        ("api_key", "API key marker"),
        ("api-key", "API key marker"),
    ]
    .into_iter()
    .find_map(|(needle, rule)| text.contains(needle).then_some(rule))
}
