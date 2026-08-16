//! Validation for language-neutral, local IPC contract fixtures.
mod frames;
mod wire;

use frames::{
    validate_chat_conversations, validate_chat_intents, validate_chat_transcript, validate_core,
    validate_event_stream, validate_handshake,
};
use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const FRAMING: &str = "u32be-length-prefixed-json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u8,
    framing: String,
    fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureEntry {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FixtureFrame {
    frame: Value,
    wire_hex: String,
}

struct FixtureSpec {
    path: &'static str,
    capability: Option<&'static str>,
}

/// Validates portable JSON fixtures without starting a daemon or provider.
///
/// # Errors
/// Returns structural and wire-contract failures with their fixture name.
pub fn validate_ipc_fixture_manifest(path: &Path) -> Result<(), String> {
    let manifest: Manifest = read_json(path)?;
    if manifest.schema_version != 1 || manifest.framing != FRAMING {
        return Err("IPC fixture manifest must be schemaVersion 1 with u32be JSON framing".into());
    }
    let entries = manifest_entries(&manifest.fixtures)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    validate_fixture(root, &entries, "handshake", validate_handshake)?;
    validate_fixture(root, &entries, "core", validate_core)?;
    validate_fixture(root, &entries, "event-stream", validate_event_stream)?;
    validate_fixture(
        root,
        &entries,
        "agent-chat-conversations",
        validate_chat_conversations,
    )?;
    validate_fixture(
        root,
        &entries,
        "agent-chat-transcript",
        validate_chat_transcript,
    )?;
    validate_fixture(root, &entries, "agent-chat-intents", validate_chat_intents)
}

fn manifest_entries(entries: &[FixtureEntry]) -> Result<BTreeMap<String, PathBuf>, String> {
    let expected = specs();
    let mut found = BTreeMap::new();
    for entry in entries {
        let Some(spec) = expected.get(entry.name.as_str()) else {
            return Err(format!("unknown IPC fixture {}", entry.name));
        };
        if entry.path != Path::new(spec.path)
            || found
                .insert(entry.name.clone(), entry.path.clone())
                .is_some()
        {
            return Err(format!("invalid or duplicate IPC fixture {}", entry.name));
        }
    }
    (found.len() == expected.len())
        .then_some(found)
        .ok_or_else(|| "IPC fixture manifest is missing a required fixture".into())
}

fn specs() -> BTreeMap<&'static str, FixtureSpec> {
    [
        (
            "handshake",
            FixtureSpec {
                path: "handshake.json",
                capability: None,
            },
        ),
        (
            "core",
            FixtureSpec {
                path: "core.json",
                capability: None,
            },
        ),
        (
            "event-stream",
            FixtureSpec {
                path: "event-stream.json",
                capability: None,
            },
        ),
        (
            "agent-chat-conversations",
            FixtureSpec {
                path: "agent-chat-conversations.json",
                capability: Some(AGENT_CHAT_CONVERSATIONS_CAPABILITY),
            },
        ),
        (
            "agent-chat-transcript",
            FixtureSpec {
                path: "agent-chat-transcript.json",
                capability: Some(AGENT_CHAT_TRANSCRIPT_CAPABILITY),
            },
        ),
        (
            "agent-chat-intents",
            FixtureSpec {
                path: "agent-chat-intents.json",
                capability: Some(AGENT_CHAT_INTENTS_CAPABILITY),
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn validate_fixture(
    root: &Path,
    entries: &BTreeMap<String, PathBuf>,
    name: &str,
    validate: fn(&[FixtureFrame]) -> Result<(), String>,
) -> Result<(), String> {
    let relative = entries
        .get(name)
        .ok_or_else(|| format!("missing IPC fixture {name}"))?;
    let value: Value = read_fixture(root, relative)?;
    let frames = fixture_frames(name, &value)?;
    validate(&frames).map_err(|error| format!("{name}: {error}"))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_fixture(root: &Path, relative: &Path) -> Result<Value, String> {
    if relative.is_absolute() || relative.components().any(|part| part.as_os_str() == "..") {
        return Err("fixture path must be manifest-relative".into());
    }
    let root =
        std::fs::canonicalize(root).map_err(|error| format!("invalid fixture root: {error}"))?;
    let path = std::fs::canonicalize(root.join(relative))
        .map_err(|error| format!("could not resolve fixture: {error}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("fixture must be a regular file inside its manifest directory".into());
    }
    read_json(&path)
}

fn fixture_frames(name: &str, value: &Value) -> Result<Vec<FixtureFrame>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "fixture must be an object".to_owned())?;
    let reserved = specs().get(name).and_then(|spec| spec.capability);
    let keys = if reserved.is_some() {
        ["schemaVersion", "state", "capability", "frames"].as_slice()
    } else {
        ["schemaVersion", "frames"].as_slice()
    };
    if object.len() != keys.len() || !keys.iter().all(|key| object.contains_key(*key)) {
        return Err("fixture has unknown or missing fields".into());
    }
    if object.get("schemaVersion") != Some(&Value::from(1)) {
        return Err("fixture schemaVersion must be 1".into());
    }
    if let Some(capability) = reserved {
        if object.get("state") != Some(&Value::from("reserved"))
            || object.get("capability") != Some(&Value::from(capability))
        {
            return Err(
                "agent-chat fixture must remain reserved behind its declared capability".into(),
            );
        }
    }
    serde_json::from_value(object["frames"].clone())
        .map_err(|error| format!("fixture frames must be wire records: {error}"))
}
