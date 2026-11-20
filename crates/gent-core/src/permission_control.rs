use gent_types::{
    AgentChatConversationId, AgentChatRunId, CommandGrant, CommandPatternError, HostEpoch,
    PermissionCategory, PermissionDecisionBinding, PermissionDecisionRequest,
    PermissionDecisionResponse, PermissionDecisionResponseKind, PermissionRequest,
    PermissionRequestDigest, PolicyRecord, SandboxEnforcement,
};

use crate::{PermissionDecision, evaluate_permission_with_sandbox};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionControlContext {
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub policy: PolicyRecord,
    pub request_digest_sha256: PermissionRequestDigest,
    pub sandbox: SandboxEnforcement,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PermissionControlState {
    pending: Option<PermissionDecisionRequest>,
}

impl PermissionControlState {
    #[must_use]
    pub fn pending(&self) -> Option<&PermissionDecisionRequest> {
        self.pending.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionControlEvent {
    Request(PermissionDecisionRequest),
    Respond(PermissionDecisionResponse),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionControlEffect {
    None,
    AskUser(PermissionDecisionRequest),
    Resolved(PermissionControlResolution),
    Rejected(PermissionControlRejection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionControlResolution {
    AllowedByPolicy(PermissionRequest),
    DeniedByPolicy(PermissionRequest),
    DeniedByUser(PermissionRequest),
    ApprovedOnce(PermissionRequest),
    ApproveExactTool { tool_name: String },
    ApproveCategory { category: PermissionCategory },
    ApproveCommandPattern { grant: CommandGrant },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionControlRejection {
    StaleConversation,
    StaleRun,
    StaleTurn,
    StaleHostEpoch,
    PolicyMismatch,
    RequestDigestMismatch,
    InvalidRequestDigest,
    PendingDecisionMismatch,
    NoPendingDecision,
    AnotherDecisionPending,
    SandboxRequired,
    CommandPattern(CommandPatternError),
}

#[must_use]
pub fn reduce_permission_control(
    state: PermissionControlState,
    context: &PermissionControlContext,
    event: PermissionControlEvent,
) -> (PermissionControlState, PermissionControlEffect) {
    match event {
        PermissionControlEvent::Request(request) => request_permission(state, context, request),
        PermissionControlEvent::Respond(response) => respond_permission(state, context, &response),
    }
}

fn request_permission(
    mut state: PermissionControlState,
    context: &PermissionControlContext,
    request: PermissionDecisionRequest,
) -> (PermissionControlState, PermissionControlEffect) {
    if let Err(rejection) = validate_binding(&request.binding, context) {
        return (state, PermissionControlEffect::Rejected(rejection));
    }
    if let Some(pending) = &state.pending {
        let effect = if pending == &request {
            PermissionControlEffect::AskUser(request)
        } else {
            PermissionControlEffect::Rejected(PermissionControlRejection::AnotherDecisionPending)
        };
        return (state, effect);
    }
    match evaluate_permission_with_sandbox(&context.policy, &request.request, context.sandbox) {
        PermissionDecision::Allow => (
            state,
            PermissionControlEffect::Resolved(PermissionControlResolution::AllowedByPolicy(
                request.request,
            )),
        ),
        PermissionDecision::Deny => (
            state,
            PermissionControlEffect::Resolved(PermissionControlResolution::DeniedByPolicy(
                request.request,
            )),
        ),
        PermissionDecision::SandboxRequired => (
            state,
            PermissionControlEffect::Rejected(PermissionControlRejection::SandboxRequired),
        ),
        PermissionDecision::Prompt => {
            state.pending = Some(request.clone());
            (state, PermissionControlEffect::AskUser(request))
        }
    }
}

fn respond_permission(
    mut state: PermissionControlState,
    context: &PermissionControlContext,
    response: &PermissionDecisionResponse,
) -> (PermissionControlState, PermissionControlEffect) {
    if let Err(rejection) = validate_binding(&response.binding, context) {
        return (state, PermissionControlEffect::Rejected(rejection));
    }
    let Some(request) = state.pending.clone() else {
        return (
            state,
            PermissionControlEffect::Rejected(PermissionControlRejection::NoPendingDecision),
        );
    };
    if request.binding != response.binding {
        return (
            state,
            PermissionControlEffect::Rejected(PermissionControlRejection::PendingDecisionMismatch),
        );
    }
    if response.response != PermissionDecisionResponseKind::ApproveCommandPattern
        && response.command_pattern.is_some()
    {
        return rejected(state, CommandPatternError::Unexpected);
    }
    let resolution = match response.response {
        PermissionDecisionResponseKind::Deny => {
            PermissionControlResolution::DeniedByUser(request.request)
        }
        PermissionDecisionResponseKind::ApproveOnce => {
            PermissionControlResolution::ApprovedOnce(request.request)
        }
        PermissionDecisionResponseKind::ApproveExactTool => {
            PermissionControlResolution::ApproveExactTool {
                tool_name: request.request.tool_name,
            }
        }
        PermissionDecisionResponseKind::ApproveCategory => {
            PermissionControlResolution::ApproveCategory {
                category: request.request.category,
            }
        }
        PermissionDecisionResponseKind::ApproveCommandPattern => {
            match gent_types::answered_command_grant(&request.request, response) {
                Ok(Some(grant)) => PermissionControlResolution::ApproveCommandPattern { grant },
                Ok(None) => return rejected(state, CommandPatternError::Missing),
                Err(error) => return rejected(state, error),
            }
        }
    };
    state.pending = None;
    (state, PermissionControlEffect::Resolved(resolution))
}

fn rejected(
    state: PermissionControlState,
    error: CommandPatternError,
) -> (PermissionControlState, PermissionControlEffect) {
    (
        state,
        PermissionControlEffect::Rejected(PermissionControlRejection::CommandPattern(error)),
    )
}

fn validate_binding(
    binding: &PermissionDecisionBinding,
    context: &PermissionControlContext,
) -> Result<(), PermissionControlRejection> {
    if binding.conversation_id != context.conversation_id {
        return Err(PermissionControlRejection::StaleConversation);
    }
    if binding.run_id != context.run_id {
        return Err(PermissionControlRejection::StaleRun);
    }
    if binding.turn_id != context.turn_id {
        return Err(PermissionControlRejection::StaleTurn);
    }
    if binding.host_epoch != context.host_epoch {
        return Err(PermissionControlRejection::StaleHostEpoch);
    }
    if binding.policy_id != context.policy.policy_id
        || binding.policy_revision != context.policy.revision
    {
        return Err(PermissionControlRejection::PolicyMismatch);
    }
    if !valid_sha256(&binding.request_digest_sha256.0) {
        return Err(PermissionControlRejection::InvalidRequestDigest);
    }
    if binding.request_digest_sha256 != context.request_digest_sha256 {
        return Err(PermissionControlRejection::RequestDigestMismatch);
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
