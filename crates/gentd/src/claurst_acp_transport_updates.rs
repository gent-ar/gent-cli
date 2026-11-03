use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{ClaurstAcpFact, ClaurstAcpTransport};

impl<S> ClaurstAcpTransport<S> {
    pub(super) fn session_update_fact(&mut self, params: Option<&Value>) -> Option<ClaurstAcpFact> {
        let update = params?.get("update")?;
        match update.get("sessionUpdate")?.as_str()? {
            "agent_message_chunk" => self.output(update),
            "agent_thought_chunk" => text(update, true),
            "tool_call" => self.tool_call(update),
            "tool_call_update" => self.tool_call_update(update),
            "usage_update" => usage(update),
            kind if kind.contains("error") || kind.contains("fail") => Some(ClaurstAcpFact::Event(
                NormalizedProviderEvent::ProviderFailure {
                    classification: gent_types::ProviderFailureClassification::Provider,
                    message: update
                        .pointer("/content/text")
                        .or_else(|| update.get("message"))
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty())
                        .map_or_else(
                            || format!("Claurst ACP session update: {kind}"),
                            str::to_owned,
                        ),
                },
            )),
            _ => None,
        }
    }

    fn output(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let text = update.get("content")?.get("text")?.as_str()?.to_owned();
        self.assistant_output.push_str(&text);
        if super::io::unanswered_stop_reason(&text).is_some() {
            return None;
        }
        Some(ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: true,
        }))
    }

    fn tool_call(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let id = tool_id(update)?;
        let name = safe_tool_name(update.get("title").and_then(Value::as_str));
        let phase = phase(update.get("status").and_then(Value::as_str)?)?;
        self.open_tools.retain(|tool| tool.id != id);
        self.open_tools.push(OpenTool {
            id: id.clone(),
            name: name.clone(),
            authority_name: update
                .get("title")
                .and_then(Value::as_str)
                .and_then(|title| title.split(':').next())
                .map(str::trim)
                .filter(|name| !name.is_empty() && name.len() <= 256)
                .map(str::to_owned),
            phase: phase.clone(),
            kind: update
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input: update.get("rawInput").cloned(),
            permission_claimed: false,
        });
        Some(tool_fact(id, name, phase, None, update.get("rawInput")))
    }

    fn tool_call_update(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let id = tool_id(update)?;
        let field = |name: &str| {
            update
                .get(name)
                .or_else(|| update.get("fields").and_then(|fields| fields.get(name)))
        };
        let phase = match field("status").and_then(Value::as_str) {
            Some(status) => phase(status)?,
            None => self
                .open_tools
                .iter()
                .find(|tool| tool.id == id)?
                .phase
                .clone(),
        };
        let input = field("rawInput").cloned().or_else(|| {
            self.open_tools
                .iter()
                .find(|tool| tool.id == id)
                .and_then(|tool| tool.input.clone())
        });
        if let Some(tool) = self.open_tools.iter_mut().find(|tool| tool.id == id) {
            tool.input.clone_from(&input);
            tool.phase = phase.clone();
        }
        let name = self
            .open_tools
            .iter()
            .find(|tool| tool.id == id)
            .map_or_else(|| "Tool".into(), |tool| tool.name.clone());
        if matches!(phase, ToolPhase::Completed | ToolPhase::Failed) {
            self.open_tools.retain(|tool| tool.id != id);
        }
        let output_digest = field("rawOutput")
            .map(|value| format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes())));
        let text = tool_output_text(field("content"), field("rawOutput"));
        let activity = tool_fact(id.clone(), name, phase, output_digest, input.as_ref());
        if text.is_empty() && field("content").is_none() && field("rawOutput").is_none() {
            return Some(activity);
        }
        self.queued.push_back(activity);
        Some(ClaurstAcpFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: id,
                text,
                is_partial: false,
            },
        ))
    }
}

pub(super) struct OpenTool {
    id: String,
    name: String,
    phase: ToolPhase,
    kind: Option<String>,
    input: Option<Value>,
    permission_claimed: bool,
    authority_name: Option<String>,
}

pub(super) struct PermissionTool {
    pub(super) tool_use_id: String,
    pub(super) tool_name: String,
    pub(super) input: Option<Value>,
}

impl<S> ClaurstAcpTransport<S> {
    pub(super) fn claim_permission_tool(&mut self, permission: &Value) -> Option<PermissionTool> {
        let id = permission.get("toolCallId")?.as_str()?;
        let title = permission.get("title")?.as_str()?;
        let input = permission.get("rawInput")?.as_object()?;
        if id.is_empty()
            || id.len() > 256
            || id.chars().any(char::is_control)
            || title.is_empty()
            || title.len() > 256
            || !title
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return None;
        }
        let name = title.to_owned();
        let input = Value::Object(input.clone());
        // One approval per actual operation in the current prompt, including
        // requests that race ahead of their display update. Bounded state.
        if self.permission_claimed_ids.len() >= 4096 || self.permission_claimed_ids.contains(id) {
            return None;
        }
        // Arrival order is not authority. A permission may precede its UI update,
        // but it must carry its actual ID and exact immutable arguments itself.
        if let Some(tool) = self.open_tools.iter_mut().find(|tool| tool.id == id) {
            if tool.permission_claimed
                || tool.authority_name.as_deref() != Some(name.as_str())
                || tool.input.as_ref().is_some_and(|known| known != &input)
            {
                return None;
            }
            tool.permission_claimed = true;
        }
        self.permission_claimed_ids.insert(id.into());
        Some(PermissionTool {
            tool_use_id: id.into(),
            tool_name: name,
            input: Some(input),
        })
    }
}

fn usage(update: &Value) -> Option<ClaurstAcpFact> {
    let used_tokens = update
        .get("used")
        .or_else(|| update.get("usedTokens"))
        .and_then(Value::as_u64)?;
    let window_tokens = update
        .get("size")
        .or_else(|| update.get("contextSize"))
        .or_else(|| update.get("contextWindow"))
        .and_then(Value::as_u64);
    Some(ClaurstAcpFact::Event(
        NormalizedProviderEvent::ContextUsage {
            used_tokens,
            window_tokens,
        },
    ))
}

fn text(update: &Value, thought: bool) -> Option<ClaurstAcpFact> {
    let text = update.get("content")?.get("text")?.as_str()?.to_owned();
    Some(if thought {
        ClaurstAcpFact::Event(NormalizedProviderEvent::Thinking {
            text,
            is_partial: true,
        })
    } else {
        ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: true,
        })
    })
}

fn tool_fact(
    id: String,
    tool_name: String,
    phase: ToolPhase,
    output_digest: Option<String>,
    input: Option<&Value>,
) -> ClaurstAcpFact {
    ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            tool_use_id: id,
            tool_name,
            phase,
            output_digest,
            input: input.and_then(gent_types::operation_preview),
            parent_tool_use_id: None,
        },
    })
}

fn tool_output_text(content: Option<&Value>, raw_output: Option<&Value>) -> String {
    let blocks = content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.pointer("/content/text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if !blocks.is_empty() {
        return blocks.join("\n");
    }
    raw_output
        .filter(|value| !value.is_null())
        .map_or_else(String::new, |value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        })
}

#[cfg(test)]
#[path = "claurst_acp_output_tests.rs"]
mod output_tests;

fn tool_id(update: &Value) -> Option<String> {
    update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
}

pub(super) fn safe_tool_name(title: Option<&str>) -> String {
    title
        .and_then(|title| title.split(':').next())
        .filter(|name| !name.trim().is_empty())
        .map_or_else(
            || "Tool".into(),
            |name| name.trim().chars().take(80).collect(),
        )
}

fn phase(status: &str) -> Option<ToolPhase> {
    match status {
        "in_progress" | "inProgress" => Some(ToolPhase::Started),
        "pending" => Some(ToolPhase::WaitingPermission),
        "completed" => Some(ToolPhase::Completed),
        "failed" => Some(ToolPhase::Failed),
        _ => None,
    }
}

#[cfg(test)]
#[path = "claurst_acp_permission_identity_tests.rs"]
mod permission_identity_tests;
