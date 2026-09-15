use gent_ports::LedgerError;
use gent_runtime::RuntimeError;
use gent_types::AgentChatRejection;

const GENERIC_REJECTION: &str = "agentChatRejected";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentChatIntentError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl From<String> for AgentChatIntentError {
    fn from(message: String) -> Self {
        Self {
            code: GENERIC_REJECTION,
            message,
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
        }
    }
}

impl From<gent_types::AgentChatCommandRejection> for AgentChatIntentError {
    fn from(rejection: gent_types::AgentChatCommandRejection) -> Self {
        Self {
            code: rejection.code(),
            message: rejection.to_string(),
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
