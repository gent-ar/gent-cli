use gent_types::NormalizedProviderEvent;
use serde_json::Value;

use super::PublicWireFact;
use super::support::string;

pub(super) fn completed_tool_output(item: &Value) -> Vec<PublicWireFact> {
    let Some(tool_use_id) = string(item, "id").filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    let output = item
        .get("aggregatedOutput")
        .or_else(|| item.get("result"))
        .or_else(|| item.get("contentItems"))
        .or_else(|| item.get("error"));
    let output = output.or_else(|| {
        (string(item, "type") == Some("fileChange"))
            .then(|| item.get("changes"))
            .flatten()
    });
    let Some(output) = output else {
        return Vec::new();
    };
    let text = match output {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(changes) if string(item, "type") == Some("fileChange") => changes
            .iter()
            .filter_map(|change| change.as_object())
            .filter_map(|change| {
                ["diff", "patch", "content"].into_iter().find_map(|key| {
                    change
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                })
            })
            .collect::<Vec<_>>()
            .join("\n"),
        value => value.to_string(),
    };
    if text.is_empty() {
        return Vec::new();
    }
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id: tool_use_id.into(),
            text,
            is_partial: false,
        },
    )]
}
