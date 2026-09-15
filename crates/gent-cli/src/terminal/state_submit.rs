use super::{
    UiCommand, UiEffect, UiRequest, UiState,
    attachments::{attach, attachment_command, existing_file_path},
    notices::notice,
    permissions, search, selection_commands, state_automations, state_documents, state_templates,
    state_thinking_commands,
};
use gent_types::{
    AgentChatClientAction as Action, AgentChatCommandDescriptor,
    AgentChatCommandDispatch as Dispatch, AgentChatCommandIntent, AgentChatCommandRejection,
    PromptTemplateVariable,
};
#[path = "state_submit_resume.rs"]
mod resume;
pub(super) fn submit(state: &mut UiState) -> UiEffect {
    if let Some(effect) = state.conversation_picker_submit() {
        return effect;
    }
    let text = state.input.trim().to_owned();
    if text.is_empty() {
        return UiEffect::Continue;
    }
    if let Some(path) = existing_file_path(&text) {
        return attach(state, path);
    }
    if let Some((name, argument)) = gent_types::slash_command(&text) {
        return command(state, name, argument);
    }
    let Some(conversation_id) = state.selected().map(|value| value.conversation_id.clone()) else {
        state.notice = Some("Create a conversation first with Ctrl+N.".into());
        return UiEffect::Continue;
    };
    let attachments = state.attachments.clone();
    UiEffect::Request(if state.turn_active() {
        UiRequest::Queue {
            conversation_id,
            text,
            attachments,
        }
    } else {
        UiRequest::Send {
            conversation_id,
            text,
            attachments,
        }
    })
}
fn command(state: &mut UiState, name: &str, argument: &str) -> UiEffect {
    let Some(catalog) = state.command_catalog() else {
        return notice(
            state,
            "Gentd has not listed this conversation's commands yet.",
        );
    };
    let Some(command) = crate::command_catalog_cli::resolve(catalog, name).cloned() else {
        let rejection = AgentChatCommandRejection::UnknownCommand { name: name.into() };
        return notice(state, &format!("{rejection}. Type / to list commands."));
    };
    match &command.dispatch {
        Dispatch::ClientAction { action } if super::super::commands::implemented(*action) => {
            client_action(state, *action, &command.name, argument)
        }
        Dispatch::ClientAction { .. } => notice(
            state,
            &format!("/{} is not available in the terminal.", command.name),
        ),
        Dispatch::Unsupported {
            reason,
            use_instead,
        } => notice(
            state,
            &AgentChatCommandRejection::UnsupportedCommand {
                name: command.name.clone(),
                reason: reason.clone(),
                use_instead: use_instead.clone(),
            }
            .to_string(),
        ),
        Dispatch::GentIntent { .. } | Dispatch::ProviderNative => invoke(state, &command, argument),
    }
}
fn invoke(state: &mut UiState, command: &AgentChatCommandDescriptor, argument: &str) -> UiEffect {
    let conversation_id = state.selected().map(|item| item.conversation_id.clone());
    if command.availability.requires_conversation && conversation_id.is_none() {
        let rejection = AgentChatCommandRejection::CommandRequiresConversation {
            name: command.name.clone(),
        };
        return notice(state, &rejection.to_string());
    }
    let creates = command.dispatch
        == Dispatch::GentIntent {
            intent: AgentChatCommandIntent::CreateConversation,
        };
    UiEffect::Request(UiRequest::InvokeCommand {
        conversation_id,
        name: command.name.clone(),
        arguments: argument.into(),
        session_id: creates.then(|| state.focused_session_id()).flatten(),
    })
}
fn client_action(state: &mut UiState, action: Action, name: &str, argument: &str) -> UiEffect {
    let slash = format!("/{name}");
    let effect = match action {
        Action::ConversationPicker => Some(resume::command(state, argument)),
        Action::Help => help_command(state, argument),
        Action::ProviderLogin => login_command(state, argument),
        Action::Attach | Action::Detach => attachment_command(state, &slash, argument),
        Action::Search => search::command(state, argument),
        Action::Thinking => Some(state_thinking_commands::command(state, argument)),
        Action::Activity => Some(state_thinking_commands::activity(state, argument)),
        Action::SteerQueued if argument.is_empty() => {
            Some(clear_then(state, UiCommand::SteerQueued))
        }
        Action::CancelQueued if argument.is_empty() => {
            Some(clear_then(state, UiCommand::CancelQueued))
        }
        Action::ContinueFromHistory if argument.is_empty() => Some(state.continue_from_history()),
        Action::Decision => permissions::command(state, &slash, argument),
        Action::PermissionSettings => permissions::settings_command(state, argument),
        Action::Automations => automation_command(state, argument),
        Action::Sessions => session_command(state, argument),
        Action::Tools => tools_command(state, argument),
        Action::Git => git_command(state, argument),
        Action::Templates if argument.is_empty() => Some(state_templates::open(state)),
        Action::Templates => template_command(state, argument),
        Action::Documents => Some(state_documents::list(state, argument)),
        Action::ApplySelection if argument.is_empty() => {
            Some(clear_then(state, UiCommand::SwitchSelection))
        }
        Action::ContextPolicy => selection_commands::command(state, "/context", argument),
        _ => None,
    };
    effect.unwrap_or_else(|| notice(state, &format!("{slash} does not take that argument.")))
}
fn login_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    let provider = match argument {
        "" => state.selection.provider,
        "claude" => gent_types::AgentChatProvider::Claude,
        "codex" => gent_types::AgentChatProvider::Codex,
        "gent" => gent_types::AgentChatProvider::Claurst,
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
fn clear_then(state: &mut UiState, command: UiCommand) -> UiEffect {
    state.input.clear();
    state.apply(command)
}
fn help_command(state: &mut UiState, argument: &str) -> Option<UiEffect> {
    if !argument.is_empty() {
        return None;
    }
    state.input.clear();
    Some(state.apply(UiCommand::ToggleHelp))
}
