use clap::Parser;
use gent_protocol::{
    AgentChatConversationFrame, AgentChatIntentFrame, AgentChatTranscriptFrame, EventStreamFrame,
    PermissionPolicyFrame, WireFrame,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const FIXTURES: [&str; 7] = [
    "handshake.json",
    "core.json",
    "event-stream.json",
    "agent-chat-conversations.json",
    "agent-chat-transcript.json",
    "agent-chat-intents.json",
    "permission-policy.json",
];

#[derive(Debug, Parser)]
#[command(about = "Refreshes deterministic u32be IPC fixture wireHex values")]
struct Args {
    #[arg(default_value = "fixtures/ipc-contract")]
    directory: PathBuf,
    #[arg(long, help = "Write refreshed fixtures in place")]
    write: bool,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    if !args.write {
        return Err("refusing to write fixtures without --write".into());
    }
    for name in FIXTURES {
        refresh(&args.directory.join(name))?;
    }
    Ok(())
}

fn refresh(path: &Path) -> Result<(), String> {
    let mut fixture: Value = read_json(path)?;
    let object = fixture
        .as_object_mut()
        .ok_or_else(|| format!("{} must be an object", path.display()))?;
    let frames = object
        .get("frames")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} frames must be an array", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let records = frames
        .iter()
        .map(|record| canonical_record(name, record.get("frame").unwrap_or(record)))
        .collect::<Result<Vec<_>, _>>()?;
    object.insert("frames".into(), Value::Array(records));
    let text = serde_json::to_string_pretty(&fixture).map_err(|error| error.to_string())?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn canonical_record(name: &str, value: &Value) -> Result<Value, String> {
    let (frame, wire_hex) = match name {
        "handshake.json" | "core.json" => canonical::<WireFrame>(value)?,
        "event-stream.json" => canonical::<EventStreamFrame>(value)?,
        "agent-chat-conversations.json" => canonical::<AgentChatConversationFrame>(value)?,
        "agent-chat-transcript.json" => canonical::<AgentChatTranscriptFrame>(value)?,
        "agent-chat-intents.json" => canonical::<AgentChatIntentFrame>(value)?,
        "permission-policy.json" => canonical::<PermissionPolicyFrame>(value)?,
        _ => return Err(format!("unknown fixture {name}")),
    };
    Ok(Value::Object(Map::from_iter([
        ("frame".into(), frame),
        ("wireHex".into(), Value::String(wire_hex)),
    ])))
}

fn canonical<T: DeserializeOwned + Serialize>(value: &Value) -> Result<(Value, String), String> {
    let frame: T = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let canonical = serde_json::to_value(&frame).map_err(|error| error.to_string())?;
    let payload = serde_json::to_vec(&frame).map_err(|error| error.to_string())?;
    let length = u32::try_from(payload.len()).map_err(|_| "frame exceeds u32 framing limit")?;
    let mut bytes = length.to_be_bytes().to_vec();
    bytes.extend(payload);
    let mut wire_hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut wire_hex, "{byte:02x}");
    }
    Ok((canonical, wire_hex))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}
