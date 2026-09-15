use gent_ports::Ledger;
use gent_protocol::{
    AgentChatIntentFrame, GoalFrame,
    agent_chat_commands::{
        AgentChatCommandFrame, CommandCatalog, CommandCatalogScope, CommandListing,
    },
};
use gent_types::{
    AgentChatCommandDescriptor, AgentChatCommandDispatch as Dispatch, AgentChatCommandIntent,
    AgentChatCommandRejection as Rejection, AgentChatConversationDetail, AgentChatConversationId,
    AgentChatRequestId, ReceiptId,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;

#[path = "runtime_facade_command_intents.rs"]
mod intents;
#[path = "runtime_facade_command_receipts.rs"]
mod receipts;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Resolved {
    Native {
        frame: AgentChatIntentFrame,
    },
    Intent {
        intent: AgentChatCommandIntent,
        frame: AgentChatIntentFrame,
    },
    Goal {
        conversation_id: AgentChatConversationId,
        frame: GoalFrame,
    },
}

struct Scope {
    detail: Option<AgentChatConversationDetail>,
    workspace_path: Option<String>,
    catalog: CommandCatalog,
}

impl RuntimeFacade {
    pub(super) fn exchange_agent_chat_command(
        &self,
        frame: AgentChatCommandFrame,
    ) -> Result<AgentChatCommandFrame, AgentChatIntentError> {
        match frame {
            AgentChatCommandFrame::ReadCommandCatalog {
                request_id,
                conversation_id,
                workspace_path,
                refresh,
            } => Ok(AgentChatCommandFrame::CommandCatalog {
                request_id,
                catalog: self
                    .command_scope(conversation_id, workspace_path, refresh)?
                    .catalog,
            }),
            AgentChatCommandFrame::InvokeCommand {
                request_id,
                receipt_id,
                conversation_id,
                workspace_path,
                name,
                arguments,
            } => self.invoke_command(
                request_id,
                receipt_id,
                conversation_id,
                workspace_path,
                &name,
                &arguments,
            ),
            _ => Err("agent-chat command responses are server-only".into()),
        }
    }

    pub(super) fn exchange_public_agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        if let AgentChatIntentFrame::SendPrompt { text, .. }
        | AgentChatIntentFrame::QueuePrompt { text, .. }
        | AgentChatIntentFrame::SendPromptWithTools { text, .. }
        | AgentChatIntentFrame::QueuePromptWithTools { text, .. } = &frame
            && let Some((name, _)) = gent_types::slash_command(text)
        {
            return Err(Rejection::SlashCommandRequiresInvoke { name: name.into() }.into());
        }
        self.exchange_agent_chat_intent(frame)
    }

    fn command_scope(
        &self,
        conversation_id: Option<AgentChatConversationId>,
        workspace_path: Option<String>,
        refresh: bool,
    ) -> Result<Scope, AgentChatIntentError> {
        let detail = conversation_id
            .as_ref()
            .map(|id| {
                self.agent_chat_reads
                    .as_ref()
                    .ok_or("agent-chat reads are unavailable")?
                    .detail(&id.0)
                    .map_err(AgentChatIntentError::from)
            })
            .transpose()?;
        let (provider, workspace_path) = match &detail {
            Some(detail) => (
                Some(detail.summary.selection.provider),
                detail.summary.workspace_path.clone(),
            ),
            None => (
                self.model_catalog
                    .as_ref()
                    .map(|catalog| catalog.default_selection().provider),
                workspace_path,
            ),
        };
        let provider_commands = match (&self.model_catalog, provider) {
            (Some(catalog), Some(provider)) => catalog.provider_commands(
                provider,
                workspace_path.as_ref().map(Into::into),
                refresh,
            ),
            _ => super::model_catalog::commands::ProviderCommands::none(),
        };
        let commands = gent_core::command_catalog(provider, provider_commands.commands);
        let digest = Sha256::digest(
            serde_json::to_vec(&(&provider_commands.listing, &commands))
                .map_err(|error| error.to_string())?,
        );
        let catalog = CommandCatalog {
            scope: CommandCatalogScope {
                conversation_id,
                workspace_path: workspace_path.clone(),
                provider,
            },
            revision: format!("sha256:{digest:x}"),
            provider_version: None,
            listing: provider_commands.listing,
            commands,
        };
        Ok(Scope {
            detail,
            workspace_path,
            catalog,
        })
    }

    fn invoke_command(
        &self,
        request_id: AgentChatRequestId,
        receipt_id: ReceiptId,
        conversation_id: Option<AgentChatConversationId>,
        workspace_path: Option<String>,
        name: &str,
        arguments: &str,
    ) -> Result<AgentChatCommandFrame, AgentChatIntentError> {
        let host_epoch = self.host_epoch()?;
        let invocation = json!({
            "conversationId": conversation_id, "workspacePath": workspace_path,
            "name": name, "arguments": arguments,
        });
        let event_id = format!("agent-chat-command:{}", receipt_id.0);
        let resolved = if let Some(resolved) = self.recorded_command(&event_id, &invocation)? {
            resolved
        } else {
            let scope = self.command_scope(conversation_id.clone(), workspace_path, false)?;
            let resolved =
                self.resolve_command(&scope, &request_id, &receipt_id, name, arguments)?;
            self.record_command(event_id, &receipt_id, host_epoch, invocation, resolved)?
        };
        let (receipt, outcome) = self.apply_command(host_epoch, &receipt_id, resolved)?;
        Ok(AgentChatCommandFrame::CommandInvoked {
            request_id,
            receipt,
            conversation_id,
            outcome,
        })
    }

    fn resolve_command(
        &self,
        scope: &Scope,
        request_id: &AgentChatRequestId,
        receipt_id: &ReceiptId,
        name: &str,
        arguments: &str,
    ) -> Result<Resolved, AgentChatIntentError> {
        let Some(command) = gent_core::resolve_command(&scope.catalog.commands, name) else {
            let name = name.to_owned();
            return Err(if scope.catalog.listing == CommandListing::Loading {
                Rejection::CommandCatalogLoading { name }
            } else {
                Rejection::UnknownCommand { name }
            }
            .into());
        };
        let name = command.name.clone();
        if command.availability.requires_conversation && scope.detail.is_none() {
            return Err(Rejection::CommandRequiresConversation { name }.into());
        }
        if let Some(detail) = &scope.detail
            && command.availability.blocked_while_turn_active
            && self.run_has_active_turn(&detail.current_run_id)?
        {
            return Err(Rejection::CommandBlockedByActiveTurn { name }.into());
        }
        match &command.dispatch {
            Dispatch::Unsupported {
                reason,
                use_instead,
            } => Err(Rejection::UnsupportedCommand {
                name,
                reason: reason.clone(),
                use_instead: use_instead.clone(),
            }
            .into()),
            Dispatch::ClientAction { .. } => Err(Rejection::ClientActionCommand { name }.into()),
            Dispatch::ProviderNative => {
                self.native_command(scope, request_id, receipt_id, command, arguments)
            }
            Dispatch::GentIntent { intent } => intents::resolve(
                self,
                *intent,
                scope,
                (request_id, receipt_id),
                &name,
                arguments,
            ),
        }
    }

    fn native_command(
        &self,
        scope: &Scope,
        request_id: &AgentChatRequestId,
        receipt_id: &ReceiptId,
        command: &AgentChatCommandDescriptor,
        arguments: &str,
    ) -> Result<Resolved, AgentChatIntentError> {
        let detail = scope
            .detail
            .as_ref()
            .ok_or("the command needs a conversation")?;
        if self
            .transcript_import_ledger
            .find_run_session_binding(&detail.current_run_id)
            .map_err(|error| error.to_string())?
            .is_none()
            && intents::conversation_has_history(self, detail)?
        {
            return Err(Rejection::CommandRequiresProviderSession {
                name: command.name.clone(),
            }
            .into());
        }
        let text = if arguments.is_empty() {
            format!("/{}", command.name)
        } else {
            format!("/{} {arguments}", command.name)
        };
        Ok(Resolved::Native {
            frame: AgentChatIntentFrame::SendPrompt {
                request_id: request_id.clone(),
                receipt_id: receipt_id.clone(),
                conversation_id: AgentChatConversationId(detail.summary.conversation_id.clone()),
                text,
                attachment_ids: Vec::new(),
            },
        })
    }

    fn run_has_active_turn(&self, run_id: &str) -> Result<bool, AgentChatIntentError> {
        Ok(
            gent_ports::ConversationLedger::list_run_turns(&self.transcript_import_ledger, run_id)
                .map_err(|error| error.to_string())?
                .iter()
                .any(|turn| !turn.phase.is_terminal()),
        )
    }
}

#[cfg(test)]
#[path = "runtime_facade_commands_tests.rs"]
mod tests;
