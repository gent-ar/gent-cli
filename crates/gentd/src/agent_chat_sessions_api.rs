use gent_protocol::AgentChatSessionFrame;
use gent_runtime::{AgentChatSessionResult, AgentChatSessionService};
use gent_store::SqliteLedger;

pub(crate) fn exchange(
    service: &AgentChatSessionService<SqliteLedger>,
    frame: AgentChatSessionFrame,
) -> Result<AgentChatSessionFrame, String> {
    match frame {
        AgentChatSessionFrame::CreateRequest {
            request_id,
            session,
        } => match service.create(session).map_err(|error| error.to_string())? {
            AgentChatSessionResult::Session(session) => Ok(AgentChatSessionFrame::Created {
                request_id,
                session,
            }),
            AgentChatSessionResult::DeniedObserver => Err("sessions are observer-disabled".into()),
            _ => Err("invalid session create result".into()),
        },
        AgentChatSessionFrame::ListRequest {
            request_id,
            workspace_id,
        } => match service
            .list(&workspace_id)
            .map_err(|error| error.to_string())?
        {
            AgentChatSessionResult::Sessions(sessions) => Ok(AgentChatSessionFrame::List {
                request_id,
                workspace_id,
                sessions,
            }),
            AgentChatSessionResult::DeniedObserver => Err("sessions are observer-disabled".into()),
            _ => Err("invalid session list result".into()),
        },
        AgentChatSessionFrame::SelectRequest {
            request_id,
            session_id,
        } => match service
            .get(&session_id)
            .map_err(|error| error.to_string())?
        {
            AgentChatSessionResult::Session(session) => Ok(AgentChatSessionFrame::Selected {
                request_id,
                session,
            }),
            AgentChatSessionResult::Missing => Err("session does not exist".into()),
            AgentChatSessionResult::DeniedObserver => Err("sessions are observer-disabled".into()),
            _ => Err("invalid session selection result".into()),
        },
        AgentChatSessionFrame::AttachRequest {
            request_id,
            session_id,
            conversation_id,
        } => match service
            .attach(&session_id, &conversation_id)
            .map_err(|error| error.to_string())?
        {
            AgentChatSessionResult::Session(session) => Ok(AgentChatSessionFrame::Attached {
                request_id,
                session,
            }),
            AgentChatSessionResult::DeniedObserver => Err("sessions are observer-disabled".into()),
            _ => Err("invalid session attach result".into()),
        },
        _ => Err("session response frames are server-only".into()),
    }
}
