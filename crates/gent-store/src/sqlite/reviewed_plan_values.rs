//! Small private codecs shared by the reviewed-plan `SQLite` authority.

use gent_ports::LedgerError;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, ContextPolicy, PlanArtifact, PlanStatus,
    StartImplementationRequest,
};
use sha2::{Digest, Sha256};

pub(super) fn decode(encoded: &str, status: &str) -> Result<PlanArtifact, LedgerError> {
    let mut plan: PlanArtifact =
        serde_json::from_str(encoded).map_err(|_| invalid("stored plan artifact"))?;
    plan.status = match status {
        "readyForReview" => PlanStatus::ReadyForReview,
        "approved" => PlanStatus::Approved,
        "rejected" => PlanStatus::Rejected,
        "superseded" => PlanStatus::Superseded,
        _ => return Err(invalid("stored plan status")),
    };
    Ok(plan)
}

pub(super) fn child_id(request: &StartImplementationRequest) -> String {
    let mut hash = Sha256::new();
    hash.update(b"gent-reviewed-plan-child-v1\0");
    hash.update(request.request_id.0.as_bytes());
    format!("reviewed-plan-run-{:x}", hash.finalize())
}

pub(super) const fn context_name(value: ContextPolicy) -> &'static str {
    match value {
        ContextPolicy::Preserve => "preserve",
        ContextPolicy::Clear => "clear",
    }
}

pub(super) const fn provider(value: AgentChatProvider) -> &'static str {
    match value {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}

pub(super) const fn effort(value: AgentChatEffort) -> &'static str {
    match value {
        AgentChatEffort::Low => "low",
        AgentChatEffort::Medium => "medium",
        AgentChatEffort::High => "high",
        AgentChatEffort::XHigh => "xhigh",
        AgentChatEffort::Max => "max",
        AgentChatEffort::Ultra => "ultra",
    }
}

pub(super) const fn mode(value: AgentChatMode) -> &'static str {
    match value {
        AgentChatMode::Ask => "ask",
        AgentChatMode::Plan => "plan",
        AgentChatMode::Agent => "agent",
    }
}

pub(super) fn invalid(subject: &str) -> LedgerError {
    LedgerError::Invariant(format!("reviewed plan {subject} is invalid"))
}
