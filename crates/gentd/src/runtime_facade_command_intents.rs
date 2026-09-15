use gent_ports::ConversationPromptLedger;
use gent_protocol::{
    AgentChatIntentFrame,
    model_catalog::{ModelCatalog, ModelCatalogFrame},
};
use gent_types::{
    AgentChatCommandIntent as Intent, AgentChatCommandRejection as Rejection,
    AgentChatConversationDetail, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy,
    ConversationMessage, ReceiptId,
};

use super::{Resolved, RuntimeFacade, Scope};
use crate::agent_chat_intent_error::AgentChatIntentError;

type Ids<'a> = (&'a AgentChatRequestId, &'a ReceiptId);

pub(super) fn resolve(
    facade: &RuntimeFacade,
    intent: Intent,
    scope: &Scope,
    (request_id, receipt_id): Ids<'_>,
    name: &str,
    arguments: &str,
) -> Result<Resolved, AgentChatIntentError> {
    let invalid = |hint: &str| -> AgentChatIntentError {
        Rejection::CommandArgumentsInvalid {
            name: name.into(),
            hint: hint.into(),
        }
        .into()
    };
    let detail = scope.detail.as_ref();
    let frame = match (intent, detail) {
        (Intent::CreateConversation, _) => {
            if !arguments.is_empty() {
                return Err(invalid("no arguments"));
            }
            AgentChatIntentFrame::CreateConversation {
                request_id: request_id.clone(),
                receipt_id: receipt_id.clone(),
                workspace_path: scope
                    .workspace_path
                    .clone()
                    .ok_or_else(|| invalid("a workspace"))?,
                selection: detail.map(|detail| detail.summary.selection.clone()),
            }
        }
        (Intent::ForkConversation, Some(detail)) => AgentChatIntentFrame::ForkConversation {
            request_id: request_id.clone(),
            receipt_id: receipt_id.clone(),
            source_conversation_id: conversation(detail),
            fork_through_message_id: if arguments.is_empty() {
                last_message(facade, detail)?
                    .ok_or_else(|| invalid("a message in this conversation to fork from"))?
                    .message_id
            } else {
                arguments.to_owned()
            },
        },
        (Intent::Goal, Some(detail)) => {
            return super::receipts::goal_frame(facade, detail, receipt_id, arguments)
                .map_err(&invalid);
        }
        (Intent::Compact, Some(detail)) => {
            if !arguments.is_empty() {
                return Err(invalid(
                    "no arguments; this provider compacts without instructions",
                ));
            }
            if !compaction_ready(facade, detail)? {
                return Err(Rejection::CommandRequiresProviderSession { name: name.into() }.into());
            }
            return Ok(Resolved::Native {
                frame: AgentChatIntentFrame::SendPrompt {
                    request_id: request_id.clone(),
                    receipt_id: receipt_id.clone(),
                    conversation_id: conversation(detail),
                    text: format!("/{name}"),
                    attachment_ids: Vec::new(),
                },
            });
        }
        (Intent::ClearContext, Some(detail)) => {
            if !arguments.is_empty() {
                return Err(invalid("no arguments"));
            }
            switch(
                request_id,
                receipt_id,
                detail,
                detail.summary.selection.clone(),
                ContextPolicy::Clear,
            )
        }
        (_, Some(detail)) => {
            let selection = selected(
                facade,
                intent,
                name,
                arguments,
                detail.summary.selection.clone(),
            )
            .map_err(&invalid)?;
            switch(
                request_id,
                receipt_id,
                detail,
                selection,
                ContextPolicy::Preserve,
            )
        }
        (_, None) => {
            return Err(Rejection::CommandRequiresConversation { name: name.into() }.into());
        }
    };
    Ok(Resolved::Intent { intent, frame })
}

fn switch(
    request_id: &AgentChatRequestId,
    receipt_id: &ReceiptId,
    detail: &AgentChatConversationDetail,
    selection: AgentChatSelection,
    context_policy: ContextPolicy,
) -> AgentChatIntentFrame {
    AgentChatIntentFrame::SwitchSelection {
        request_id: request_id.clone(),
        receipt_id: receipt_id.clone(),
        conversation_id: conversation(detail),
        parent_run_id: AgentChatRunId(detail.current_run_id.clone()),
        selection,
        context_policy,
    }
}

fn selected(
    facade: &RuntimeFacade,
    intent: Intent,
    name: &str,
    arguments: &str,
    mut selection: AgentChatSelection,
) -> Result<AgentChatSelection, &'static str> {
    let catalog = model_catalog(facade);
    let value = arguments.to_ascii_lowercase();
    match intent {
        Intent::SelectProvider => {
            selection.provider = match value.as_str() {
                "claude" => AgentChatProvider::Claude,
                "codex" => AgentChatProvider::Codex,
                "gent" => AgentChatProvider::Claurst,
                _ => return Err("claude, codex, or gent"),
            };
            selection.model = catalog
                .as_ref()
                .and_then(|catalog| {
                    catalog
                        .providers
                        .iter()
                        .find(|entry| entry.provider == selection.provider)
                })
                .and_then(|entry| {
                    entry
                        .models
                        .iter()
                        .find(|model| model.is_default)
                        .or_else(|| entry.models.first())
                })
                .map(|model| model.id.clone())
                .ok_or("a provider whose models Gent has listed")?;
        }
        Intent::SelectModel if !arguments.is_empty() => arguments.clone_into(&mut selection.model),
        Intent::SelectModel => return Err("a model identifier"),
        Intent::SelectEffort => {
            selection.effort = crate::runtime_facade::model_catalog::service::effort(&value)
                .ok_or("low, medium, high, xhigh, max, or ultra")?;
            return Ok(selection);
        }
        Intent::SelectMode => {
            selection.mode = match (name, value.as_str()) {
                ("plan", "") | (_, "plan") => AgentChatMode::Plan,
                (_, "ask") => AgentChatMode::Ask,
                (_, "agent") => AgentChatMode::Agent,
                _ => return Err("ask, plan, or agent"),
            };
            return Ok(selection);
        }
        _ => return Err("a supported command"),
    }
    selection.effort = fitted_effort(catalog.as_ref(), &selection);
    Ok(selection)
}

fn fitted_effort(
    catalog: Option<&ModelCatalog>,
    selection: &AgentChatSelection,
) -> AgentChatEffort {
    let Some(model) = catalog
        .and_then(|catalog| {
            catalog
                .providers
                .iter()
                .find(|entry| entry.provider == selection.provider)
        })
        .and_then(|entry| {
            entry
                .models
                .iter()
                .find(|model| model.id == selection.model)
        })
    else {
        return selection.effort;
    };
    if model.efforts.is_empty() || model.efforts.contains(&selection.effort) {
        return selection.effort;
    }
    model
        .default_effort
        .or_else(|| model.efforts.first().copied())
        .unwrap_or(selection.effort)
}

fn model_catalog(facade: &RuntimeFacade) -> Option<ModelCatalog> {
    match facade
        .model_catalog
        .as_ref()?
        .exchange(ModelCatalogFrame::ReadModelCatalog {
            request_id: "agent-chat-command".into(),
            refresh: false,
        }) {
        Ok(ModelCatalogFrame::ModelCatalog { catalog, .. }) => Some(catalog),
        _ => None,
    }
}

fn compaction_ready(
    facade: &RuntimeFacade,
    detail: &AgentChatConversationDetail,
) -> Result<bool, AgentChatIntentError> {
    if detail.summary.selection.provider == AgentChatProvider::Claurst {
        return conversation_has_history(facade, detail);
    }
    Ok(gent_ports::Ledger::find_run_session_binding(
        &facade.transcript_import_ledger,
        &detail.current_run_id,
    )
    .map_err(|error| error.to_string())?
    .is_some())
}

pub(super) fn conversation_has_history(
    facade: &RuntimeFacade,
    detail: &AgentChatConversationDetail,
) -> Result<bool, AgentChatIntentError> {
    Ok(!facade
        .agent_chat_reads
        .as_ref()
        .ok_or("agent-chat reads are unavailable")?
        .transcript(&detail.summary.conversation_id, None, 1)?
        .events
        .is_empty())
}

fn last_message(
    facade: &RuntimeFacade,
    detail: &AgentChatConversationDetail,
) -> Result<Option<ConversationMessage>, AgentChatIntentError> {
    let mut run_id = Some(detail.current_run_id.clone());
    while let Some(current) = run_id {
        if let Some(message) = run_messages(facade, &current)?
            .into_iter()
            .max_by_key(|message| message.sequence)
        {
            return Ok(Some(message));
        }
        run_id = detail
            .runs
            .iter()
            .find(|run| run.run_id == current)
            .and_then(|run| run.parent_run_id.clone());
    }
    Ok(None)
}

fn run_messages(
    facade: &RuntimeFacade,
    run_id: &str,
) -> Result<Vec<ConversationMessage>, AgentChatIntentError> {
    facade
        .transcript_import_ledger
        .list_run_messages(run_id)
        .map_err(|error| error.to_string().into())
}

pub(super) fn conversation(detail: &AgentChatConversationDetail) -> AgentChatConversationId {
    AgentChatConversationId(detail.summary.conversation_id.clone())
}
