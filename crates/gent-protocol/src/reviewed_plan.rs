//! Capability-gated IPC for reading a reviewed plan and explicitly approving or rejecting it.
//!
//! This contract never starts provider work itself. The observer daemon deliberately does not
//! advertise this capability until an authority runtime composes the matching service.

use gent_types::{
    AgentChatConversationId, PlanArtifact, PlanRevision, ReviewedPlanId,
    StartImplementationRequest, StartImplementationResult,
};
use serde::{Deserialize, Serialize};

/// Required before a client may read or settle a reviewed implementation plan.
pub const REVIEWED_PLAN_CAPABILITY: &str = "reviewed-plan-v1";

/// One finite, correlated reviewed-plan exchange.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ReviewedPlanFrame {
    ReviewRead {
        request_id: String,
        conversation_id: AgentChatConversationId,
        plan_id: ReviewedPlanId,
    },
    Review {
        request_id: String,
        plan: Option<PlanArtifact>,
    },
    /// Explicit user approval of one exact plan revision and context policy.
    StartImplementation { request: StartImplementationRequest },
    StartedImplementation {
        request_id: String,
        result: StartImplementationResult,
    },
    /// Explicit user rejection of one exact immutable plan revision.
    Reject {
        request_id: String,
        plan_id: ReviewedPlanId,
        plan_revision: PlanRevision,
        plan_content_digest_sha256: String,
    },
    Rejected {
        request_id: String,
        plan_id: ReviewedPlanId,
        plan_revision: PlanRevision,
    },
}

impl ReviewedPlanFrame {
    /// Validates only public correlation and reviewed-plan values before transport.
    ///
    /// # Errors
    /// Returns an error when a frame has malformed correlation or plan values.
    pub fn validate(&self) -> Result<(), ReviewedPlanFrameError> {
        match self {
            Self::ReviewRead {
                request_id,
                conversation_id,
                plan_id,
            } => {
                valid_id(request_id)?;
                valid_id(&conversation_id.0)?;
                valid_id(&plan_id.0)?;
            }
            Self::Review { request_id, plan } => {
                valid_id(request_id)?;
                if let Some(plan) = plan {
                    plan.validate()?;
                }
            }
            Self::StartImplementation { request } => request.validate()?,
            Self::StartedImplementation { request_id, result } => {
                valid_id(request_id)?;
                valid_start_result(result)?;
            }
            Self::Reject {
                request_id,
                plan_id,
                plan_revision,
                plan_content_digest_sha256,
            } => {
                valid_id(request_id)?;
                valid_id(&plan_id.0)?;
                valid_revision_digest(*plan_revision, plan_content_digest_sha256)?;
            }
            Self::Rejected {
                request_id,
                plan_id,
                plan_revision,
            } => {
                valid_id(request_id)?;
                valid_id(&plan_id.0)?;
                if plan_revision.0 == 0 {
                    return Err(ReviewedPlanFrameError::InvalidPlan);
                }
            }
        }
        Ok(())
    }
}

fn valid_start_result(result: &StartImplementationResult) -> Result<(), ReviewedPlanFrameError> {
    valid_id(&result.conversation_id.0)?;
    valid_id(&result.plan_id.0)?;
    valid_id(&result.parent_run_id.0)?;
    valid_id(&result.implementation_run_id.0)?;
    if result.plan_revision.0 == 0 {
        return Err(ReviewedPlanFrameError::InvalidPlan);
    }
    Ok(())
}

fn valid_revision_digest(
    revision: PlanRevision,
    digest: &str,
) -> Result<(), ReviewedPlanFrameError> {
    if revision.0 == 0
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ReviewedPlanFrameError::InvalidPlan);
    }
    Ok(())
}

fn valid_id(value: &str) -> Result<(), ReviewedPlanFrameError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(ReviewedPlanFrameError::InvalidIdentifier);
    }
    Ok(())
}

/// Deliberately value-free protocol validation errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReviewedPlanFrameError {
    #[error("reviewed-plan correlation identifier is invalid")]
    InvalidIdentifier,
    #[error("reviewed-plan value is invalid")]
    InvalidPlan,
}

impl From<gent_types::ReviewedPlanContractError> for ReviewedPlanFrameError {
    fn from(_: gent_types::ReviewedPlanContractError) -> Self {
        Self::InvalidPlan
    }
}

#[cfg(test)]
#[path = "reviewed_plan_tests.rs"]
mod tests;
