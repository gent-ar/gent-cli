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
            "usage_update" | "current_usage" => usage(update),
            kind if kind.contains("error") || kind.contains("fail") => {
                Some(ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
                    text: format!("Claurst ACP session update: {update}"),
                    is_partial: false,
                }))
            }
            _ => None,
        }
    }

    fn output(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let text = update.get("content")?.get("text")?.as_str()?.to_owned();
        self.assistant_output.push_str(&text);
        Some(ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text,
            is_partial: true,
        }))
    }

    fn tool_call(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let id = tool_id(update)?;
        let name = safe_tool_name(update.get("title").and_then(Value::as_str));
        self.tool_names.insert(id.clone(), name.clone());
        Some(tool_fact(
            id,
            name,
            phase(update.get("status").and_then(Value::as_str)?)?,
            None,
        ))
    }

    fn tool_call_update(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let id = tool_id(update)?;
        let status = update
            .get("fields")
            .and_then(|fields| fields.get("status"))
            .and_then(Value::as_str)?;
        let phase = phase(status)?;
        let name = self
            .tool_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "Tool".into());
        if matches!(phase, ToolPhase::Completed | ToolPhase::Failed) {
            self.tool_names.remove(&id);
        }
        let output_digest = if matches!(phase, ToolPhase::Completed | ToolPhase::Failed) {
            update
                .get("rawOutput")
                .or_else(|| update.pointer("/fields/rawOutput"))
                .map(|value| format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes())))
        } else {
            None
        };
        Some(tool_fact(id, name, phase, output_digest))
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
) -> ClaurstAcpFact {
    ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
        activity: ToolActivity {
            tool_use_id: id,
            tool_name,
            phase,
            output_digest,
        },
    })
}

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
