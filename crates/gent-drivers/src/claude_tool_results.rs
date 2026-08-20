//! Runner-owned correlation for Claude result blocks that omit the original tool name.

use std::collections::BTreeMap;

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::public_protocol::PublicWireFact;

pub(crate) fn remember(facts: &[PublicWireFact], tool_names: &mut BTreeMap<String, String>) {
    for fact in facts {
        let PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) = fact
        else {
            continue;
        };
        if activity.phase == ToolPhase::Started {
            tool_names
                .entry(activity.tool_use_id.clone())
                .or_insert_with(|| activity.tool_name.clone());
        }
    }
}

pub(crate) fn results(
    tool_names: &mut BTreeMap<String, String>,
    frame: &Value,
) -> Option<Vec<PublicWireFact>> {
    let content = frame
        .pointer("/message/content")
        .and_then(Value::as_array)?;
    Some(
        content
            .iter()
            .filter(|block| string(block, "type") == Some("tool_result"))
            .flat_map(|block| result(tool_names, block))
            .collect(),
    )
}

fn result(tool_names: &mut BTreeMap<String, String>, block: &Value) -> Vec<PublicWireFact> {
    let Some(tool_use_id) = string(block, "tool_use_id").filter(|value| !value.is_empty()) else {
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
    let output_digest = block
        .get("content")
        .map(|value| format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes())));
    vec![PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: tool_use_id.into(),
                tool_name,
                phase,
                output_digest,
            },
        },
    )]
}

fn diagnostic(classification: &str) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    )]
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}
