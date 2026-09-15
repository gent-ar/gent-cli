//! Runner-owned correlation for Claude result blocks that omit the original tool name.

use std::collections::BTreeMap;

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::public_protocol::PublicWireFact;

pub(crate) fn remember(
    facts: &[PublicWireFact],
    started_tools: &mut BTreeMap<String, ToolActivity>,
) {
    for fact in facts {
        let PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) = fact
        else {
            continue;
        };
        if activity.phase == ToolPhase::Started {
            started_tools
                .entry(activity.tool_use_id.clone())
                .or_insert_with(|| activity.clone());
        }
    }
}

pub(crate) fn results(
    started_tools: &mut BTreeMap<String, ToolActivity>,
    frame: &Value,
) -> Option<Vec<PublicWireFact>> {
    let content = frame
        .pointer("/message/content")
        .and_then(Value::as_array)?;
    let top_level = frame.get("parent_tool_use_id").is_none_or(Value::is_null);
    Some(
        content
            .iter()
            .filter(|block| string(block, "type") == Some("tool_result"))
            .flat_map(|block| {
                let activity = result(started_tools, block);
                let output = top_level.then(|| output(block)).flatten();
                std::iter::once(activity).chain(output)
            })
            .collect(),
    )
}

fn output(block: &Value) -> Option<PublicWireFact> {
    let tool_use_id = string(block, "tool_use_id").filter(|value| !value.is_empty())?;
    let text = crate::public_protocol::claude_protocol::content_text(block.get("content")?);
    (!text.is_empty()).then(|| {
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id: tool_use_id.into(),
            text,
            is_partial: false,
        })
    })
}

fn result(started_tools: &mut BTreeMap<String, ToolActivity>, block: &Value) -> PublicWireFact {
    let Some(tool_use_id) = string(block, "tool_use_id").filter(|value| !value.is_empty()) else {
        return diagnostic("malformedClaudeToolResult");
    };
    let Some(started) = started_tools.remove(tool_use_id) else {
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
    PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            phase,
            output_digest,
            ..started
        },
    })
}

fn diagnostic(classification: &str) -> PublicWireFact {
    PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
        classification: classification.into(),
    })
}
fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gent_types::{NormalizedProviderEvent, ToolActivity, ToolPhase};
    use serde_json::json;

    use super::results;
    use crate::public_protocol::PublicWireFact;

    fn started() -> BTreeMap<String, ToolActivity> {
        BTreeMap::from([(
            "tool-1".to_owned(),
            ToolActivity {
                tool_use_id: "tool-1".into(),
                tool_name: "Read".into(),
                phase: ToolPhase::Started,
                output_digest: None,
                parent_tool_use_id: None,
            },
        )])
    }

    #[test]
    fn a_named_top_level_result_keeps_its_output_for_later_providers() {
        let facts = results(
            &mut started(),
            &json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{
                "type":"tool_result","tool_use_id":"tool-1","content":"The vault token is V-1."
            }]}}),
        )
        .unwrap();
        assert!(facts.iter().any(|fact| matches!(
            fact,
            PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { tool_use_id, text, is_partial: false })
                if tool_use_id == "tool-1" && text == "The vault token is V-1."
        )));
    }

    #[test]
    fn subagent_internal_results_stay_out_of_the_parent_transcript() {
        let facts = results(
            &mut started(),
            &json!({"type":"user","parent_tool_use_id":"task-1","message":{"content":[{
                "type":"tool_result","tool_use_id":"tool-1","content":"child detail"
            }]}}),
        )
        .unwrap();
        assert_eq!(facts.len(), 1);
    }
}
