use std::sync::Arc;

use agent_client_protocol_schema as acp;
use claurst_core::permissions::{PermissionDecision, PermissionRequest};
use claurst_core::PermissionHandler;
use claurst_tools::PendingPermissionRequest;
use tracing::{debug, warn};

use crate::connection::Connection;
use crate::sessions::SessionState;

pub struct AcpPermissionHandler;

impl PermissionHandler for AcpPermissionHandler {
    fn check_permission(&self, _request: &PermissionRequest) -> PermissionDecision {
        PermissionDecision::Ask {
            reason: String::new(),
        }
    }

    fn request_permission(&self, request: &PermissionRequest) -> PermissionDecision {
        let mut reason = format!("Tool '{}' requires approval", request.tool_name);
        if let Some(detail) = &request.details {
            reason.push_str(": ");
            reason.push_str(detail);
        }
        PermissionDecision::Ask { reason }
    }
}

pub(crate) async fn forward_pending(
    connection: Arc<Connection>,
    session: Arc<SessionState>,
    pending: PendingPermissionRequest,
) {
    let PendingPermissionRequest {
        tool_use_id,
        request,
        decision_tx,
        call,
        parent_tool_call_id,
        ..
    } = pending;

    let Some(decision_tx) = decision_tx.filter(|tx| !tx.is_closed()) else {
        debug!(tool_use_id, "ACP permission: requester stopped waiting");
        return;
    };

    let Some(call) = call else {
        warn!(
            tool_use_id,
            "ACP permission: request is not bound to a tool call, denying"
        );
        let _ = decision_tx.send(PermissionDecision::Deny);
        return;
    };

    let parent = parent_tool_call_id.as_deref();
    crate::prompt::announce_tool_call(
        &connection,
        &session,
        &call.name,
        &call.id,
        Some(call.input.clone()),
        parent,
    )
    .await;

    let fields = acp::ToolCallUpdateFields::new()
        .kind(Some(infer_tool_kind(&request)))
        .status(Some(acp::ToolCallStatus::Pending))
        .title(Some(call.name))
        .raw_input(Some(call.input));
    let tool_call = acp::ToolCallUpdate::new(acp::ToolCallId::new(call.id.as_str()), fields)
        .meta(parent.map(crate::prompt::child_meta));

    let options = vec![
        acp::PermissionOption::new(
            acp::PermissionOptionId::new("allow_once"),
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        ),
        acp::PermissionOption::new(
            acp::PermissionOptionId::new("allow_always"),
            "Allow always",
            acp::PermissionOptionKind::AllowAlways,
        ),
        acp::PermissionOption::new(
            acp::PermissionOptionId::new("reject_once"),
            "Reject",
            acp::PermissionOptionKind::RejectOnce,
        ),
    ];

    let request_params =
        acp::RequestPermissionRequest::new(session.session_id.clone(), tool_call, options);

    debug!(tool = %request.tool_name, "ACP permission: requesting from client");
    let result = connection
        .send_request::<_, acp::RequestPermissionResponse>(
            "session/request_permission",
            request_params,
        )
        .await;

    let decision = match result {
        Ok(Ok(response)) => match response.outcome {
            acp::RequestPermissionOutcome::Selected(sel) => match sel.option_id.0.as_ref() {
                "allow_once" => PermissionDecision::Allow,
                "allow_always" => PermissionDecision::AllowPermanently,
                "reject_always" => PermissionDecision::DenyPermanently,
                _ => PermissionDecision::Deny,
            },
            _ => PermissionDecision::Deny,
        },
        Ok(Err(err)) => {
            warn!(?err, "ACP permission: client returned error, denying");
            PermissionDecision::Deny
        }
        Err(err) => {
            warn!(?err, "ACP permission: send_request failed, denying");
            PermissionDecision::Deny
        }
    };

    let _ = decision_tx.send(decision);
}

fn infer_tool_kind(request: &PermissionRequest) -> acp::ToolKind {
    if request.is_read_only {
        return acp::ToolKind::Read;
    }
    crate::prompt::classify_tool_kind(&request.tool_name)
}

pub(crate) fn spawn_forwarder(connection: Arc<Connection>, session: Arc<SessionState>) {
    let ready = session.pending_permissions.lock().ready.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = connection.closed_token.cancelled() => return,
                _ = ready.notified() => {}
            }
            let popped: Vec<PendingPermissionRequest> =
                session.pending_permissions.lock().queue.drain(..).collect();
            for pending in popped {
                tokio::spawn(forward_pending(
                    connection.clone(),
                    session.clone(),
                    pending,
                ));
            }
        }
    });
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;
