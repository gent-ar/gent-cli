use gent_types::{
    AgentChatCommandAvailability, AgentChatCommandDescriptor, AgentChatCommandDispatch,
    AgentChatCommandOrigin, AgentChatProvider, MAX_COMMAND_TEXT_BYTES, valid_command_name,
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaudeBuiltin {
    Native,
    Unsupported(&'static str, Option<&'static str>),
    GentReserved,
}

use ClaudeBuiltin::{GentReserved, Native, Unsupported};

const PROVIDER_SETTINGS: &str =
    "Claude settings diverge from Gent-owned selection, MCP, and permission state.";

#[rustfmt::skip]
pub const CLAUDE_BUILTIN_COMMANDS: &[(&str, ClaudeBuiltin)] = &[
    ("advisor", Unsupported("The advisor model changes Claude's selection outside Gent.", Some("/model"))),
    ("agents", Unsupported("Claude subagent configuration is not managed through Gent.", None)),
    ("auto-mode-setup", Unsupported("Permission posture is Gent-owned.", Some("/permissions"))),
    ("autocompact", Unsupported("Claude compaction settings are provider configuration Gent does not track.", None)),
    ("batch", Native),
    ("claude-api", Native),
    ("clear", GentReserved),
    ("code-review", Native),
    ("color", Unsupported("Session colors only apply to Claude's own terminal.", None)),
    ("compact", Native),
    ("config", Unsupported(PROVIDER_SETTINGS, None)),
    ("context", Native),
    ("dataviz", Native),
    ("debug", Native),
    ("deep-research", Native),
    ("design", Native),
    ("design-consent", Native),
    ("design-revoke", Native),
    ("design-sync", Native),
    ("doctor", Unsupported("Gent verifies provider installations itself.", None)),
    ("effort", GentReserved),
    ("fast", Unsupported("Fast mode changes Claude's selection outside Gent.", Some("/model"))),
    ("fewer-permission-prompts", Unsupported("Permission rules are Gent-owned.", Some("/permissions"))),
    ("goal", GentReserved),
    ("heapdump", Unsupported("Claude process diagnostics are not exposed through Gent.", None)),
    ("init", Native),
    ("insights", Native),
    ("list-agents", Native),
    ("loop", Unsupported("Recurring provider turns would bypass Gent's turn ledger.", Some("/goal"))),
    ("mcp", Unsupported("MCP servers are configured through Gent.", Some("/tools"))),
    ("model", GentReserved),
    ("output-style", Unsupported("Output styles change Claude's system prompt outside Gent.", None)),
    ("recap", Native),
    ("reload-plugins", Unsupported("Plugin changes are provider configuration Gent does not track.", None)),
    ("reload-skills", Native),
    ("rename", GentReserved),
    ("run", Native),
    ("run-skill-generator", Native),
    ("security-review", Native),
    ("simplify", Native),
    ("team-onboarding", Native),
    ("update-config", Unsupported(PROVIDER_SETTINGS, None)),
    ("usage", Native),
    ("verify", Native),
    ("workflow-authoring", Native),
    ("workflow-launch-exec", Native),
];

pub fn claude_builtin(name: &str) -> Option<ClaudeBuiltin> {
    CLAUDE_BUILTIN_COMMANDS
        .iter()
        .find(|(builtin, _)| *builtin == name)
        .map(|(_, disposition)| *disposition)
}

pub fn claude_command_descriptors(initialize: &Value) -> Option<Vec<AgentChatCommandDescriptor>> {
    let entries = initialize.get("commands")?.as_array()?;
    Some(entries.iter().filter_map(descriptor).collect())
}

fn descriptor(entry: &Value) -> Option<AgentChatCommandDescriptor> {
    let name = entry.get("name")?.as_str()?.trim_start_matches('/');
    if !valid_command_name(name) || name.starts_with("__") {
        return None;
    }
    let provider = AgentChatProvider::Claude;
    let (origin, dispatch) = match claude_builtin(name) {
        Some(GentReserved) => return None,
        Some(Native) => (
            AgentChatCommandOrigin::ProviderBuiltin { provider },
            AgentChatCommandDispatch::ProviderNative,
        ),
        Some(Unsupported(reason, use_instead)) => (
            AgentChatCommandOrigin::ProviderBuiltin { provider },
            AgentChatCommandDispatch::Unsupported {
                reason: reason.into(),
                use_instead: use_instead.map(str::to_owned),
            },
        ),
        None => (
            AgentChatCommandOrigin::ProviderSkill { provider },
            AgentChatCommandDispatch::ProviderNative,
        ),
    };
    let native = dispatch == AgentChatCommandDispatch::ProviderNative;
    Some(AgentChatCommandDescriptor {
        name: name.into(),
        aliases: entry
            .get("aliases")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|alias| valid_command_name(alias) && *alias != name)
            .map(str::to_owned)
            .collect(),
        description: text(entry.get("description"))
            .unwrap_or_else(|| format!("Run /{name} in Claude")),
        argument_hint: text(entry.get("argumentHint")),
        origin,
        dispatch,
        availability: AgentChatCommandAvailability {
            requires_conversation: native,
            blocked_while_turn_active: native,
        },
    })
}

fn text(value: Option<&Value>) -> Option<String> {
    let line = value?.as_str()?.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let mut end = line.len().min(MAX_COMMAND_TEXT_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Some(
        line[..end]
            .chars()
            .filter(|character| !character.is_control())
            .collect(),
    )
}

#[cfg(test)]
#[path = "claude_commands_tests.rs"]
mod tests;
