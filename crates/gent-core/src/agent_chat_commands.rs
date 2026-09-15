use gent_types::{
    AgentChatClientAction as Action, AgentChatCommandAvailability, AgentChatCommandDescriptor,
    AgentChatCommandDispatch, AgentChatCommandIntent as Intent, AgentChatCommandOrigin,
    AgentChatProvider,
};

enum Kind {
    Intent(Intent),
    Client(Action),
    Compact,
}

#[rustfmt::skip]
struct Row(&'static str, &'static [&'static str], &'static str, Option<&'static str>, Kind, bool);

#[rustfmt::skip]
const GENT_COMMANDS: &[Row] = &[
    Row("new", &[], "Start a new conversation", None, Kind::Intent(Intent::CreateConversation), false),
    Row("clear", &["reset"], "Continue in a new run with cleared context", None, Kind::Intent(Intent::ClearContext), true),
    Row("fork", &[], "Fork this conversation into a new one", None, Kind::Intent(Intent::ForkConversation), true),
    Row("provider", &[], "Switch provider", Some("<claude|codex|gent>"), Kind::Intent(Intent::SelectProvider), true),
    Row("model", &[], "Switch model", Some("<model>"), Kind::Intent(Intent::SelectModel), true),
    Row("effort", &[], "Switch effort", Some("<low|medium|high|xhigh|max|ultra>"), Kind::Intent(Intent::SelectEffort), true),
    Row("mode", &[], "Switch mode", Some("<ask|plan|agent>"), Kind::Intent(Intent::SelectMode), true),
    Row("plan", &[], "Switch to plan mode", None, Kind::Intent(Intent::SelectMode), true),
    Row("goal", &[], "Set, pause, resume, or clear a goal", Some("<objective|pause|resume|clear>"), Kind::Intent(Intent::Goal), true),
    Row("compact", &[], "Compact the conversation context", None, Kind::Compact, true),
    Row("resume", &[], "Open a conversation", Some("[conversation-id]"), Kind::Client(Action::ConversationPicker), false),
    Row("help", &[], "Show commands and shortcuts", None, Kind::Client(Action::Help), false),
    Row("login", &[], "Sign in to a provider", Some("[claude|codex]"), Kind::Client(Action::ProviderLogin), false),
    Row("rename", &[], "Rename this conversation", Some("[name]"), Kind::Client(Action::Rename), true),
    Row("btw", &[], "Ask a side question without interrupting", Some("<question>"), Kind::Client(Action::SideQuestion), true),
    Row("attach", &[], "Attach a file", Some("<path>"), Kind::Client(Action::Attach), false),
    Row("detach", &[], "Clear pending files", None, Kind::Client(Action::Detach), false),
    Row("search", &[], "Filter conversations", Some("<text|clear>"), Kind::Client(Action::Search), false),
    Row("thinking", &[], "Show or hide provider thinking", Some("<show|hide|toggle>"), Kind::Client(Action::Thinking), false),
    Row("activity", &[], "Open live activity", None, Kind::Client(Action::Activity), false),
    Row("steer", &[], "Send queued prompts into the running turn", None, Kind::Client(Action::SteerQueued), true),
    Row("cancel-queued", &[], "Remove the last queued prompt", None, Kind::Client(Action::CancelQueued), true),
    Row("continue", &[], "Continue a failed turn from saved history", None, Kind::Client(Action::ContinueFromHistory), true),
    Row("approve", &[], "Approve the pending request once", None, Kind::Client(Action::Decision), true),
    Row("approve-tool", &[], "Always approve this tool", None, Kind::Client(Action::Decision), true),
    Row("approve-category", &[], "Always approve this category", None, Kind::Client(Action::Decision), true),
    Row("deny", &[], "Deny the pending request", None, Kind::Client(Action::Decision), true),
    Row("answer", &[], "Answer the pending question", Some("<json>"), Kind::Client(Action::Decision), true),
    Row("permissions", &[], "Change workspace permissions", Some("<ask|edits|autonomous|bypass confirm>"), Kind::Client(Action::PermissionSettings), true),
    Row("automations", &["automation"], "Open or run automations", Some("[run <id>]"), Kind::Client(Action::Automations), false),
    Row("session", &[], "Create a durable session", Some("<name>"), Kind::Client(Action::Sessions), true),
    Row("tools", &[], "List configured MCP servers", None, Kind::Client(Action::Tools), false),
    Row("git", &[], "Show workspace Git status", None, Kind::Client(Action::Git), true),
    Row("templates", &["template"], "Open or render prompt templates", Some("[<id> name=value…]"), Kind::Client(Action::Templates), false),
    Row("documents", &["attach-doc"], "Stage a workspace document", Some("[filter]"), Kind::Client(Action::Documents), true),
    Row("switch", &[], "Apply the selected provider, model, effort, and mode", None, Kind::Client(Action::ApplySelection), true),
    Row("context-policy", &[], "Choose preserved or cleared context for switches", Some("<preserve|clear>"), Kind::Client(Action::ContextPolicy), false),
];

pub fn command_catalog(
    provider: Option<AgentChatProvider>,
    provider_commands: impl IntoIterator<Item = AgentChatCommandDescriptor>,
) -> Vec<AgentChatCommandDescriptor> {
    let mut catalog: Vec<AgentChatCommandDescriptor> = GENT_COMMANDS
        .iter()
        .filter_map(|row| descriptor(row, provider))
        .collect();
    let reserved = reserved_names(provider);
    for mut command in provider_commands {
        if command.name.starts_with("__")
            || reserved.contains(&command.name.as_str())
            || catalog
                .iter()
                .any(|existing| existing.answers_to(&command.name))
        {
            continue;
        }
        command
            .aliases
            .retain(|alias| !catalog.iter().any(|existing| existing.answers_to(alias)));
        catalog.push(command);
    }
    catalog
}

pub fn reserved_names(provider: Option<AgentChatProvider>) -> Vec<&'static str> {
    GENT_COMMANDS
        .iter()
        .filter(|row| descriptor(row, provider).is_some())
        .flat_map(|Row(name, aliases, ..)| std::iter::once(*name).chain(aliases.iter().copied()))
        .collect()
}

pub fn resolve_command<'a>(
    catalog: &'a [AgentChatCommandDescriptor],
    name: &str,
) -> Option<&'a AgentChatCommandDescriptor> {
    catalog.iter().find(|command| command.answers_to(name))
}

fn descriptor(
    row: &Row,
    provider: Option<AgentChatProvider>,
) -> Option<AgentChatCommandDescriptor> {
    let Row(name, aliases, description, hint, kind, requires_conversation) = row;
    let (dispatch, blocked_while_turn_active) = match (kind, provider) {
        (Kind::Intent(intent), _) => (
            AgentChatCommandDispatch::GentIntent { intent: *intent },
            !matches!(intent, Intent::CreateConversation | Intent::Goal),
        ),
        (Kind::Client(action), _) => (
            AgentChatCommandDispatch::ClientAction { action: *action },
            false,
        ),
        (Kind::Compact, Some(AgentChatProvider::Claude)) => return None,
        (Kind::Compact, _) => (
            AgentChatCommandDispatch::GentIntent {
                intent: Intent::Compact,
            },
            true,
        ),
    };
    Some(AgentChatCommandDescriptor {
        name: (*name).into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        description: (*description).into(),
        argument_hint: hint.map(str::to_owned),
        origin: AgentChatCommandOrigin::Gent,
        dispatch,
        availability: AgentChatCommandAvailability {
            requires_conversation: *requires_conversation,
            blocked_while_turn_active,
        },
    })
}

#[cfg(test)]
#[path = "agent_chat_commands_tests.rs"]
mod tests;
