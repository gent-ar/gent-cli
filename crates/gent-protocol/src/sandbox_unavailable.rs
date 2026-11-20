use gent_types::{AgentChatProvider, Containment};
use serde::{Deserialize, Serialize};

pub const SANDBOX_UNAVAILABLE: &str = "sandboxUnavailable";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxUnavailable {
    pub provider: AgentChatProvider,
    pub containment: Containment,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SandboxRejectionFrame {
    Error {
        code: String,
        message: String,
        sandbox: SandboxUnavailable,
    },
}

#[cfg(test)]
mod tests {
    use super::{SANDBOX_UNAVAILABLE, SandboxRejectionFrame, SandboxUnavailable};
    use gent_types::{AgentChatFailureCode, AgentChatProvider, Containment};
    use serde_json::json;

    #[test]
    fn an_unenforceable_sandbox_names_the_provider_and_containment() {
        let frame = SandboxRejectionFrame::Error {
            code: SANDBOX_UNAVAILABLE.into(),
            message: "no sandbox".into(),
            sandbox: SandboxUnavailable {
                provider: AgentChatProvider::Codex,
                containment: Containment::Sandboxed,
            },
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({"type": "error", "body": {
                "code": "sandboxUnavailable",
                "message": "no sandbox",
                "sandbox": {"provider": "codex", "containment": "sandboxed"}
            }})
        );
        assert_eq!(
            serde_json::to_value(AgentChatFailureCode::SandboxUnavailable).unwrap(),
            json!(SANDBOX_UNAVAILABLE)
        );
    }
}
