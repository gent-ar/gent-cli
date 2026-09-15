use gent_types::{AgentChatEffort, AgentChatMode};
use serde_json::json;

use crate::local_model_catalog::{LocalAgentProfile, LocalModelRecord};

const COMPACT_TOOLS: [&str; 6] = ["Glob", "Grep", "Read", "Edit", "Write", "Bash"];
const COMPACT_READ_ONLY_TOOLS: [&str; 3] = ["Glob", "Grep", "Read"];
const COMPACT_GUIDANCE: &str = "\
- Work step by step with tools until you can answer; do not stop after one tool call if the answer needs more.
- Never describe what a file or project does without reading it. To explain a project: 1) Glob \"**/*\" to list files, 2) Read README.md, 3) Read the main source files, 4) only then answer.
- Never guess file types, extensions, or paths. If a tool finds nothing or fails, fix the input and try again.
- For Edit, old_string must appear exactly once: include the whole line or neighbouring lines.
- Do not use tools for questions you can answer from the conversation. After using tools, reply to the user with the answer.";
const BYTES_PER_TOKEN: usize = 3;
const TURN_RESERVE_TOKENS: u32 = 1_024;

pub(super) fn template_kwargs(
    model: &LocalModelRecord,
    effort: AgentChatEffort,
    mode: AgentChatMode,
) -> String {
    let mut kwargs = json!({ "gent_instructions": instructions(model, effort, mode) });
    if model.agent_profile == LocalAgentProfile::Compact {
        kwargs["gent_tools"] = match mode {
            AgentChatMode::Agent => json!(COMPACT_TOOLS),
            AgentChatMode::Ask | AgentChatMode::Plan => json!(COMPACT_READ_ONLY_TOOLS),
        };
    }
    kwargs.to_string()
}

pub(super) fn history_input_bytes(model: &LocalModelRecord, effort: AgentChatEffort) -> usize {
    let overhead = match model.agent_profile {
        LocalAgentProfile::Compact => 2_400,
        LocalAgentProfile::Full => 8_200,
    };
    let available = model
        .context_tokens
        .saturating_sub(overhead + local_max_tokens(effort) + TURN_RESERVE_TOKENS);
    usize::try_from(available)
        .unwrap_or(usize::MAX)
        .saturating_mul(BYTES_PER_TOKEN)
        .clamp(8 * 1024, 512 * 1024)
}

fn instructions(model: &LocalModelRecord, effort: AgentChatEffort, mode: AgentChatMode) -> String {
    let guidance =
        if model.agent_profile == LocalAgentProfile::Compact && mode == AgentChatMode::Agent {
            format!("\nTools: {}.\n{COMPACT_GUIDANCE}", COMPACT_TOOLS.join(", "))
        } else {
            String::new()
        };
    format!(
        "{}{guidance}{}",
        mode_instruction(mode),
        qwen3_effort_instruction(model, effort)
    )
}

fn mode_instruction(mode: AgentChatMode) -> &'static str {
    match mode {
        AgentChatMode::Ask => "Answer and explain. Do not invoke tools or change files.",
        AgentChatMode::Plan => {
            "Inspect only as needed, then provide a concrete plan. Do not change files or invoke destructive tools."
        }
        AgentChatMode::Agent => {
            "You are Gent, a local coding agent working in the user's project directory. Use the available tools when needed. Request permission before actions that require it."
        }
    }
}

fn qwen3_effort_instruction(model: &LocalModelRecord, effort: AgentChatEffort) -> &'static str {
    if !model.id.starts_with("qwen3-") {
        return "";
    }
    match effort {
        AgentChatEffort::Low | AgentChatEffort::Medium => {
            "\n\nAnswer directly without a separate thinking phase."
        }
        AgentChatEffort::High
        | AgentChatEffort::XHigh
        | AgentChatEffort::Max
        | AgentChatEffort::Ultra => {
            "\n\nReason carefully, then always finish with a direct answer or completed action."
        }
    }
}

pub(super) fn qwen3_reasoning_arguments(
    model: &LocalModelRecord,
    effort: AgentChatEffort,
) -> Vec<String> {
    if !model.id.starts_with("qwen3-") {
        return Vec::new();
    }
    match effort {
        AgentChatEffort::Low | AgentChatEffort::Medium => vec![
            "--reasoning".into(),
            "off".into(),
            "--reasoning-budget".into(),
            "0".into(),
        ],
        AgentChatEffort::High
        | AgentChatEffort::XHigh
        | AgentChatEffort::Max
        | AgentChatEffort::Ultra => vec![
            "--reasoning".into(),
            "on".into(),
            "--reasoning-effort".into(),
            reasoning_effort(effort).into(),
            "--reasoning-budget".into(),
            reasoning_budget(effort).to_string(),
            "--reasoning-budget-message".into(),
            "Reasoning budget reached. Finish with a direct answer now.".into(),
        ],
    }
}

const fn reasoning_effort(effort: AgentChatEffort) -> &'static str {
    match effort {
        AgentChatEffort::Low => "minimal",
        AgentChatEffort::Medium => "medium",
        AgentChatEffort::High => "high",
        AgentChatEffort::XHigh => "xhigh",
        AgentChatEffort::Max | AgentChatEffort::Ultra => "max",
    }
}

const fn reasoning_budget(effort: AgentChatEffort) -> u32 {
    match effort {
        AgentChatEffort::Low | AgentChatEffort::Medium => 0,
        AgentChatEffort::High => 1_024,
        AgentChatEffort::XHigh => 1_536,
        AgentChatEffort::Max => 2_048,
        AgentChatEffort::Ultra => 3_072,
    }
}

pub(super) fn local_max_tokens(effort: AgentChatEffort) -> u32 {
    match effort {
        AgentChatEffort::Low => 2_048,
        AgentChatEffort::Medium => 4_096,
        AgentChatEffort::High => 8_192,
        AgentChatEffort::XHigh => 12_288,
        AgentChatEffort::Max => 16_384,
        AgentChatEffort::Ultra => 24_576,
    }
}
