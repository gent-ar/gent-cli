use gent_ports::LedgerError;
use gent_protocol::sandbox_unavailable::{
    SANDBOX_UNAVAILABLE, SandboxRejectionFrame, SandboxUnavailable,
};
use gent_protocol::workspace_unavailable::{
    WORKSPACE_UNAVAILABLE, WorkspaceRejectionFrame, WorkspaceUnavailable,
};
use gent_runtime::RuntimeError;
use gent_types::AgentChatRejection;

const GENERIC_REJECTION: &str = "agentChatRejected";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentChatIntentError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) details: Option<RejectionDetails>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RejectionDetails {
    Command(Box<gent_types::AgentChatCommandRejectionDetails>),
    Interrupt(Box<gent_protocol::agent_chat_interrupt::InterruptTurnEnded>),
    Workspace(WorkspaceUnavailable),
    Sandbox(SandboxUnavailable),
}

impl AgentChatIntentError {
    pub(crate) fn coded(code: &'static str, message: &str) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub(crate) fn sandbox_unavailable(sandbox: SandboxUnavailable, reason: String) -> Self {
        Self {
            code: SANDBOX_UNAVAILABLE,
            message: reason,
            details: Some(RejectionDetails::Sandbox(sandbox)),
        }
    }

    pub(crate) async fn write<S>(
        &self,
        stream: &mut S,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: tokio::io::AsyncWrite + Unpin,
    {
        match &self.details {
            Some(RejectionDetails::Command(command)) => {
                let frame =
                    gent_protocol::agent_chat_commands::AgentChatCommandRejectionFrame::Error {
                        code: self.code.into(),
                        message: self.message.clone(),
                        command: command.as_ref().clone(),
                    };
                gent_protocol::write_json_frame(stream, &frame).await?;
                Ok(())
            }
            Some(RejectionDetails::Interrupt(interrupt)) => {
                let frame = gent_protocol::agent_chat_interrupt::InterruptRejectionFrame::Error {
                    code: self.code.into(),
                    message: self.message.clone(),
                    interrupt: interrupt.as_ref().clone(),
                };
                gent_protocol::write_json_frame(stream, &frame).await?;
                Ok(())
            }
            Some(RejectionDetails::Workspace(workspace)) => {
                let frame = WorkspaceRejectionFrame::Error {
                    code: self.code.into(),
                    message: self.message.clone(),
                    workspace: workspace.clone(),
                };
                gent_protocol::write_json_frame(stream, &frame).await?;
                Ok(())
            }
            Some(RejectionDetails::Sandbox(sandbox)) => {
                let frame = SandboxRejectionFrame::Error {
                    code: self.code.into(),
                    message: self.message.clone(),
                    sandbox: sandbox.clone(),
                };
                gent_protocol::write_json_frame(stream, &frame).await?;
                Ok(())
            }
            None => crate::transport::write_error(stream, self.code, &self.message).await,
        }
    }
}

impl From<WorkspaceUnavailable> for AgentChatIntentError {
    fn from(workspace: WorkspaceUnavailable) -> Self {
        Self {
            code: WORKSPACE_UNAVAILABLE,
            message: format!(
                "the workspace folder {} does not exist or is not a directory",
                workspace.path
            ),
            details: Some(RejectionDetails::Workspace(workspace)),
        }
    }
}

impl From<gent_protocol::agent_chat_interrupt::InterruptTurnEnded> for AgentChatIntentError {
    fn from(interrupt: gent_protocol::agent_chat_interrupt::InterruptTurnEnded) -> Self {
        Self {
            code: gent_protocol::agent_chat_interrupt::INTERRUPT_TURN_ENDED,
            message: "the interrupted turn already ended".into(),
            details: Some(RejectionDetails::Interrupt(Box::new(interrupt))),
        }
    }
}

impl From<String> for AgentChatIntentError {
    fn from(message: String) -> Self {
        Self {
            code: GENERIC_REJECTION,
            message,
            details: None,
        }
    }
}

impl From<&str> for AgentChatIntentError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl From<AgentChatRejection> for AgentChatIntentError {
    fn from(rejection: AgentChatRejection) -> Self {
        Self {
            code: rejection.code(),
            message: rejection.to_string(),
            details: None,
        }
    }
}

impl From<gent_types::AgentChatCommandRejection> for AgentChatIntentError {
    fn from(rejection: gent_types::AgentChatCommandRejection) -> Self {
        Self {
            code: rejection.code(),
            message: rejection.to_string(),
            details: Some(RejectionDetails::Command(Box::new(rejection.details()))),
        }
    }
}

impl From<RuntimeError> for AgentChatIntentError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::Ledger(LedgerError::Rejected(rejection)) => rejection.into(),
            error => error.to_string().into(),
        }
    }
}

impl From<AgentChatIntentError> for String {
    fn from(error: AgentChatIntentError) -> Self {
        error.message
    }
}
