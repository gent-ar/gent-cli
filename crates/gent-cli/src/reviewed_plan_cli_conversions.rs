use gent_protocol::ReviewedPlanFrame;
use gent_types::{AgentChatEffort, AgentChatProvider, AgentChatSelection};

use crate::chat_cli::Provider;

pub(super) fn implementation_selection(
    planning: &AgentChatSelection,
    provider: Option<Provider>,
    model: Option<String>,
    effort: Option<AgentChatEffort>,
) -> Result<AgentChatSelection, String> {
    let provider = provider.map(|provider| match provider {
        Provider::Claude => AgentChatProvider::Claude,
        Provider::Codex => AgentChatProvider::Codex,
        Provider::Gent => AgentChatProvider::Claurst,
    });
    if provider.is_some_and(|provider| provider != planning.provider) && model.is_none() {
        return Err("--model is required when --provider changes".into());
    }
    Ok(AgentChatSelection {
        provider: provider.unwrap_or(planning.provider),
        model: model.unwrap_or_else(|| planning.model.clone()),
        effort: effort.unwrap_or(planning.effort),
        mode: planning.mode,
    })
}

pub(super) fn valid_reply(request: &ReviewedPlanFrame, response: &ReviewedPlanFrame) -> bool {
    match (request, response) {
        (
            ReviewedPlanFrame::ReviewRead { request_id, .. },
            ReviewedPlanFrame::Review {
                request_id: reply, ..
            },
        ) => reply == request_id,
        (
            ReviewedPlanFrame::StartImplementation { request },
            ReviewedPlanFrame::StartedImplementation { request_id, result },
        ) => request_id == &request.request_id.0 && result.receipt.receipt_id == request.receipt_id,
        (
            ReviewedPlanFrame::Reject {
                request_id,
                plan_id,
                plan_revision,
                ..
            },
            ReviewedPlanFrame::Rejected {
                request_id: reply,
                plan_id: rejected,
                plan_revision: revision,
            },
        ) => reply == request_id && rejected == plan_id && revision == plan_revision,
        _ => false,
    }
}
