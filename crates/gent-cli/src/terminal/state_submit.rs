use super::{
    UiCommand, UiEffect, UiRequest, UiState, notices::notice, permissions, search,
    selection_commands, state_automations, state_documents, state_templates,
    state_thinking_commands,
};
use gent_types::{ContextPolicy, PromptTemplateVariable};
pub(super) fn submit(state: &mut UiState) -> UiEffect {
    let text = state.input.trim().to_owned();
    if text.is_empty() {
        return UiEffect::Continue;
    }
    if let Some(path) = existing_file_path(&text) {
        return attach(state, path);
    }
    if let Some(effect) = slash_command(state, &text) {
        return effect;
    }
    let Some(conversation_id) = state.selected().map(|value| value.conversation_id.clone()) else {
        state.notice = Some("Create a conversation first with Ctrl+N.".into());
        return UiEffect::Continue;
    };
    match goal_summary(&text) {
        Ok(Some(summary)) => {
            let Some(run_id) = state.parent_run_id.clone() else {
                state.notice =
                    Some("Run status is unavailable; refusing to guess a /goal binding.".into());
                return UiEffect::Continue;
            };
            state.input.clear();
            return UiEffect::Request(UiRequest::Goal {
                conversation_id,
                run_id,
                summary,
            });
        }
        Ok(None) => {}
        Err(notice) => {
            state.notice = Some(notice.into());
            return UiEffect::Continue;
        }
    }
    UiEffect::Request(UiRequest::Send {
        conversation_id,
        text,
        attachments: state.attachments.clone(),
    })
}
fn slash_command(state: &mut UiState, text: &str) -> Option<UiEffect> {
    let (command, argument) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(head, tail)| (head, tail.trim()));
    match command {
        "/new" => new_command(state, argument),
        "/resume" => Some(resume_command(state, argument)),
        "/provider" | "/model" | "/effort" | "/mode" | "/plan" | "/context" => {
            selection_commands::command(state, command, argument)
        }
        "/switch" | "/fork" | "/clear" => switch_command(state, command, argument),
        "/attach" | "/detach" => attachment_command(state, command, argument),
        "/approve" | "/approve-tool" | "/approve-category" | "/deny" | "/answer" => {
            permissions::command(state, command, argument)
        }
        "/permissions" => permissions::settings_command(state, argument),
        "/login" => login_command(state, argument),
        "/automation" => automation_command(state, argument),
        "/automations" => Some(state_automations::open(state)),
        "/session" => session_command(state, argument),
        "/tools" => tools_command(state, argument),
        "/git" => git_command(state, argument),
        "/template" => template_command(state, argument),
        "/documents" | "/attach-doc" => Some(state_documents::list(state, argument)),
        "/templates" => Some(state_templates::open(state)),
        "/search" => search::command(state, argument),
        "/thinking" => Some(state_thinking_commands::command(state, argument)),
        "/activity" => Some(state_thinking_commands::activity(state, argument)),
        "/help" => help_command(state, argument),
        _ => None,
    }
}
fn login_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    let provider = match argument {
        "" => state.selection.provider,
        "claude" => gent_types::AgentChatProvider::Claude,
        "codex" => gent_types::AgentChatProvider::Codex,
        "gent" | "claurst" => gent_types::AgentChatProvider::Claurst,
        _ => return Some(notice(state, "/login accepts claude or codex.")),
    };
    if provider == gent_types::AgentChatProvider::Claurst {
        return Some(notice(
            state,
            "Gent uses the selected local model; no account login is needed.",
        ));
    }
    state.input.clear();
    Some(UiEffect::Login(provider))
}
fn session_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    let name = argument.trim();
    if name.is_empty() {
        return Some(notice(state, "/session NAME creates a durable session."));
    }
    state.input.clear();
    Some(state.create_session(name))
}
fn template_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    let mut parts = argument.split_whitespace();
    let Some(template_id) = parts.next() else {
        return Some(notice(
            state,
            "/template requires an ID and optional name=value variables.",
        ));
    };
    let mut variables = Vec::new();
    for value in parts {
        let Some((name, value)) = value.split_once('=') else {
            return Some(notice(state, "Template variables must use name=value."));
        };
        if name.is_empty() {
            return Some(notice(state, "Template variable names cannot be empty."));
        }
        variables.push(PromptTemplateVariable {
            name: name.into(),
            value: value.into(),
        });
    }
    state.input.clear();
    Some(UiEffect::RenderTemplate {
        template_id: template_id.into(),
        variables,
    })
}
fn automation_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if argument.is_empty() {
        return Some(state_automations::open(state));
    }
    let Some(automation_id) = argument.strip_prefix("run ").map(str::trim) else {
        return Some(notice(
            state,
            "/automation run ID starts a backed manual automation.",
        ));
    };
    if automation_id.is_empty() {
        return Some(notice(state, "/automation run requires an automation ID."));
    }
    let Some(conversation_id) = state.selected().map(|item| item.conversation_id.clone()) else {
        return Some(notice(
            state,
            "Select a conversation before running an automation.",
        ));
    };
    state.input.clear();
    Some(UiEffect::Request(UiRequest::RunAutomation {
        automation_id: automation_id.into(),
        conversation_id,
    }))
}
fn tools_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if argument.is_empty() {
        state.input.clear();
        let names = state.selected_mcp_server_names();
        state.notice = Some(if names.is_empty() {
            "No MCP servers are configured for this workspace.".into()
        } else {
            format!(
                "MCP servers: {}. All configured servers are active by default.",
                names.join(", ")
            )
        });
        return Some(UiEffect::Continue);
    }
    state.input.clear();
    state.notice = Some(
        "Configured MCP servers are active for this conversation. Use /tools to review them."
            .into(),
    );
    Some(UiEffect::Continue)
}

fn git_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if !argument.is_empty() {
        return Some(notice(state, "/git does not take an argument."));
    }
    state.input.clear();
    let workspace = state.selected_workspace_path().unwrap_or("No workspace");
    let branch = state.selected_git_branch().unwrap_or("no branch");
    let files = state.selected_changed_file_count().map_or_else(
        || "change count unavailable".into(),
        |count| format!("{count} changed"),
    );
    state.notice = Some(format!("Git · {branch} · {files} · {workspace}"));
    Some(UiEffect::Continue)
}
fn new_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if !argument.is_empty() {
        return None;
    }
    state.input.clear();
    Some(UiEffect::Request(UiRequest::Create {
        selection: state.selection.clone(),
        session_id: state.focused_session_id(),
    }))
}
fn resume_command(state: &mut UiState, argument: &str) -> UiEffect {
    if let Some((candidate, prompt)) = argument.split_once(char::is_whitespace)
        && state.select_conversation(candidate)
    {
        let prompt = prompt.trim();
        state.input.clear();
        return if prompt.is_empty() {
            UiEffect::Refresh(candidate.into())
        } else {
            UiEffect::Request(UiRequest::Send {
                conversation_id: candidate.into(),
                text: prompt.into(),
                attachments: Vec::new(),
            })
        };
    }
    if !argument.is_empty() && state.select_conversation(argument) {
        state.input.clear();
        return UiEffect::Refresh(argument.into());
    }
    let Some(conversation_id) = state.selected().map(|value| value.conversation_id.clone()) else {
        state.notice = Some("Select a conversation before using /resume.".into());
        return UiEffect::Continue;
    };
    state.input.clear();
    if argument.is_empty() {
        UiEffect::Refresh(conversation_id)
    } else {
        UiEffect::Request(UiRequest::Send {
            conversation_id,
            text: argument.to_owned(),
            attachments: Vec::new(),
        })
    }
}
fn attachment_command(state: &mut UiState, command: &str, argument: &str) -> Option<UiEffect> {
    if command == "/detach" {
        if !argument.is_empty() {
            return None;
        }
        let count = state.attachments.len();
        state.attachments.clear();
        state.input.clear();
        state.notice = Some(format!("Removed {count} pending attachment(s)."));
        return Some(UiEffect::Continue);
    }
    if argument.is_empty() {
        return Some(notice(state, "/attach requires a file path."));
    }
    Some(match attachment_path(argument) {
        Some(path) => attach(state, path),
        None => notice(state, "Attach requires a local file path."),
    })
}
pub(super) fn paste(state: &mut UiState, value: String) -> UiEffect {
    if let Some(path) = existing_file_path(&value) {
        attach(state, path)
    } else {
        state.input.push_str(&value);
        UiEffect::Continue
    }
}
fn existing_file_path(value: &str) -> Option<std::path::PathBuf> {
    let path = attachment_path(value)?;
    path.is_file().then_some(path)
}

fn attachment_path(value: &str) -> Option<std::path::PathBuf> {
    let value = value.trim().trim_matches('"').replace("\\ ", " ");
    if let Some(value) = value.strip_prefix("file://") {
        return file_url_path(value).map(std::path::PathBuf::from);
    }
    (!value.is_empty()).then(|| std::path::PathBuf::from(value))
}

fn file_url_path(value: &str) -> Option<String> {
    let path = if value.starts_with('/') {
        value.to_owned()
    } else {
        format!("/{}", value.strip_prefix("localhost/")?)
    };
    let mut bytes = Vec::with_capacity(path.len());
    let source = path.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let high = *source.get(index + 1)?;
            let low = *source.get(index + 2)?;
            bytes.push((hex(high)? << 4) | hex(low)?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

const fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
fn attach(state: &mut UiState, path: std::path::PathBuf) -> UiEffect {
    if !path.is_file() {
        return notice(state, "Attach requires a readable file path.");
    }
    if state.attachments.iter().any(|value| value == &path) {
        return notice(state, "That file is already attached.");
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file")
        .to_owned();
    state.attachments.push(path);
    state.input.clear();
    state.notice = Some(format!("Attached {name}. Enter sends it with the prompt."));
    UiEffect::Continue
}
fn switch_command(state: &mut UiState, command: &str, argument: &str) -> Option<UiEffect> {
    if !argument.is_empty() {
        return None;
    }
    if command == "/clear" {
        state.context_policy = ContextPolicy::Clear;
    }
    state.input.clear();
    Some(state.apply(UiCommand::SwitchSelection))
}
fn help_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if !argument.is_empty() {
        return None;
    }
    state.input.clear();
    Some(state.apply(UiCommand::ToggleHelp))
}
fn goal_summary(text: &str) -> Result<Option<String>, &'static str> {
    let Some(summary) = text.strip_prefix("/goal") else {
        return Ok(None);
    };
    if !summary.is_empty() && !summary.chars().next().is_some_and(char::is_whitespace) {
        return Ok(None);
    }
    let summary = summary.trim();
    (!summary.is_empty())
        .then(|| Some(summary.to_owned()))
        .ok_or("`/goal` requires a concise summary; no provider work was started")
}
