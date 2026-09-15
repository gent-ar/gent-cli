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
        self.open_tools.retain(|tool| tool.id != id);
        self.open_tools.push(OpenTool {
            id: id.clone(),
            name: name.clone(),
            kind: update
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input: update.get("rawInput").cloned(),
            permission_claimed: false,
        });
        Some(tool_fact(
            id,
            name,
            phase(update.get("status").and_then(Value::as_str)?)?,
            None,
        ))
    }

    fn tool_call_update(&mut self, update: &Value) -> Option<ClaurstAcpFact> {
        let id = tool_id(update)?;
        let field = |name: &str| {
            update
                .get(name)
                .or_else(|| update.get("fields").and_then(|fields| fields.get(name)))
        };
        let status = field("status").and_then(Value::as_str)?;
        let phase = phase(status)?;
        let name = self
            .open_tools
            .iter()
            .find(|tool| tool.id == id)
            .map_or_else(|| "Tool".into(), |tool| tool.name.clone());
        if matches!(phase, ToolPhase::Completed | ToolPhase::Failed) {
            self.open_tools.retain(|tool| tool.id != id);
        }
        if !matches!(phase, ToolPhase::Completed | ToolPhase::Failed) {
            return Some(tool_fact(id, name, phase, None));
        }
        let output_digest = field("rawOutput")
            .map(|value| format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes())));
        let text = tool_output_text(field("content"), field("rawOutput"));
        let activity = tool_fact(id.clone(), name, phase, output_digest);
        if text.is_empty() {
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
    kind: Option<String>,
    input: Option<Value>,
    permission_claimed: bool,
}

pub(super) struct PermissionTool {
    pub(super) tool_use_id: String,
    pub(super) tool_name: String,
    pub(super) input: Option<Value>,
}

impl<S> ClaurstAcpTransport<S> {
    pub(super) fn claim_permission_tool(&mut self, permission: &Value) -> Option<PermissionTool> {
        let named = permission
            .get("title")
            .and_then(Value::as_str)
            .and_then(|title| title.strip_prefix("Tool '"))
            .and_then(|rest| rest.split_once('\''))
            .map(|(name, _)| name);
        let kind = permission.get("kind").and_then(Value::as_str);
        let unclaimed = || {
            self.open_tools
                .iter()
                .enumerate()
                .filter(|(_, tool)| !tool.permission_claimed)
        };
        let only = || {
            let mut tools = unclaimed();
            tools.next().filter(|_| tools.next().is_none())
        };
        let (index, _) = unclaimed()
            .find(|(_, tool)| Some(tool.name.as_str()) == named)
            .or_else(|| {
                unclaimed().find(|(_, tool)| {
                    kind.is_some_and(|kind| same_kind(kind, tool.kind.as_deref()))
                })
            })
            .or_else(only)?;
        let tool = &mut self.open_tools[index];
        tool.permission_claimed = true;
        Some(PermissionTool {
            tool_use_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            input: tool.input.clone(),
        })
    }
}

fn same_kind(permission: &str, tool: Option<&str>) -> bool {
    tool == Some(permission) || (permission == "read" && tool == Some("search"))
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
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
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
