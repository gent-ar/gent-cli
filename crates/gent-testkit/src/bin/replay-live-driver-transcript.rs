//! Replays a raw, just-captured provider transcript through the real parser.
//!
//! Reads one JSON provider frame per line from stdin and feeds every line through
//! `gent_drivers::public_protocol::normalize_public_frame`, failing loudly if any frame
//! produces a `TransportDiagnostic` fact (an unrecognized or malformed shape the parser
//! could not classify). This is the local, on-demand counterpart to the redacted
//! committed fixtures: those never contain real provider output (by design, for
//! redaction), so they cannot catch a CLI wire-format change. This binary is meant to
//! run against freshly captured, ephemeral, local-only output — never commit its input.
//!
//! `normalize_public_frame` is intentionally stateless (see `codex_turn.rs`); the real
//! runners (`claude_runner.rs`, `codex_turn.rs`) layer correlation on top of it for a few
//! frame shapes that omit context by design — most importantly, real Claude tool-result
//! blocks omit the tool name, which the runner resolves from an earlier `tool_use` fact
//! (see `claude_tool_results.rs`). This binary replicates that one piece of Claude
//! correlation locally (it is simple and stable) so a genuinely new/renamed frame type
//! isn't drowned out by an expected, by-design "unresolved" diagnostic. It does NOT
//! replicate Codex's child/parent thread correlation or either vendor's permission
//! control-request protocol — those are runner-owned state this tool doesn't reconstruct,
//! so `control_request`/`control_cancel_request` frames are skipped rather than flagged.
//!
//! The `codex` vendor here expects `codex app-server`-shaped (`method`-keyed JSON-RPC)
//! notification frames — the only shape `codex_protocol.rs` understands, and the only
//! transport gentd's real Codex driver ever spawns (see `codex_runner.rs`). Captured via
//! `public_driver_codex_appserver.py`, a minimal app-server JSON-RPC client built
//! specifically for this check; `codex exec --json` (what every other tool in this repo
//! drives) emits an unrelated `type`-keyed shape and is never fed here.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};

use clap::Parser;
use gent_drivers::PublicProvider;
use gent_drivers::public_protocol::{PublicWireFact, normalize_public_frame};
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(
    about = "Replays a freshly captured raw provider transcript through the real parser and fails on any TransportDiagnostic"
)]
struct Args {
    #[arg(value_enum)]
    vendor: Vendor,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum Vendor {
    Claude,
    Codex,
}

impl From<Vendor> for PublicProvider {
    fn from(vendor: Vendor) -> Self {
        match vendor {
            Vendor::Claude => Self::Claude,
            Vendor::Codex => Self::Codex,
        }
    }
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    if std::io::stdin().is_terminal() {
        return Err("pass raw JSONL provider frames on stdin, one JSON value per line".into());
    }
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|error| error.to_string())?;
    let frames: Vec<Value> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|error| format!("invalid JSON line: {error}"))
        })
        .collect::<Result<_, _>>()?;
    if frames.is_empty() {
        return Err("stdin contained no JSON lines to replay".into());
    }
    let facts = match args.vendor {
        Vendor::Claude => replay_claude(&frames),
        Vendor::Codex => frames
            .iter()
            .flat_map(|frame| normalize_public_frame(PublicProvider::Codex, frame))
            .collect(),
    };
    let diagnostics: Vec<&str> = facts
        .iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification,
            }) => Some(classification.as_str()),
            _ => None,
        })
        .collect();
    println!(
        "replayed {} frame(s) into {} fact(s)",
        frames.len(),
        facts.len()
    );
    if diagnostics.is_empty() {
        println!("no unrecognized frame shapes");
        return Ok(());
    }
    for classification in &diagnostics {
        println!("UNRECOGNIZED: {classification}");
    }
    Err(format!(
        "{} frame(s) were not recognized by the current parser \u{2014} the CLI's wire format may have drifted",
        diagnostics.len()
    ))
}

/// Mirrors `claude_runner.rs`'s dispatch for the pieces this tool can check without a full
/// live runner: skips permission control frames, resolves tool-result names the same way
/// `claude_tool_results.rs` does, and otherwise replays through the stateless normalizer.
fn replay_claude(frames: &[Value]) -> Vec<PublicWireFact> {
    let mut tool_names: BTreeMap<String, String> = BTreeMap::new();
    let mut facts = Vec::new();
    for frame in frames {
        match frame.get("type").and_then(Value::as_str) {
            Some("control_request" | "control_cancel_request") => continue,
            Some("user") => {
                facts.extend(claude_user_frame(frame, &mut tool_names));
            }
            _ => {
                let frame_facts = normalize_public_frame(PublicProvider::Claude, frame);
                remember_tool_names(&frame_facts, &mut tool_names);
                facts.extend(frame_facts);
            }
        }
    }
    facts
}

fn claude_user_frame(
    frame: &Value,
    tool_names: &mut BTreeMap<String, String>,
) -> Vec<PublicWireFact> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .map(|block| claude_tool_result(block, tool_names))
        .collect()
}

fn claude_tool_result(block: &Value, tool_names: &mut BTreeMap<String, String>) -> PublicWireFact {
    let diagnostic = |classification: &str| {
        PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        })
    };
    let Some(tool_use_id) = block
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return diagnostic("malformedClaudeToolResult");
    };
    let Some(tool_name) = tool_names.remove(tool_use_id) else {
        return diagnostic("unresolvedClaudeToolResult");
    };
    let phase = if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        ToolPhase::Failed
    } else {
        ToolPhase::Completed
    };
    PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            tool_use_id: tool_use_id.into(),
            tool_name,
            phase,
            output_digest: None,
        },
    })
}

fn remember_tool_names(facts: &[PublicWireFact], tool_names: &mut BTreeMap<String, String>) {
    for fact in facts {
        if let PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) =
            fact
        {
            if activity.phase == ToolPhase::Started {
                tool_names
                    .entry(activity.tool_use_id.clone())
                    .or_insert_with(|| activity.tool_name.clone());
            }
        }
    }
}
