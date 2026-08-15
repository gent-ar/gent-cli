//! Credential-free values shared by a private external-provider bridge contract.

use serde::{Deserialize, Serialize};

/// A private bridge session identifier. Its value is opaque to Gent's public runtime.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderSession {
    pub run_id: String,
    pub opaque_session: String,
}

/// A terminal result reported by a private external-provider bridge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalProviderTerminal {
    Completed,
    Interrupted,
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::{ExternalProviderSession, ExternalProviderTerminal};

    #[test]
    fn bridge_values_use_camel_case_without_provider_configuration() {
        let session = ExternalProviderSession {
            run_id: "run-1".into(),
            opaque_session: "opaque-1".into(),
        };
        let terminal = ExternalProviderTerminal::Failed {
            message: "stopped".into(),
        };

        let session = serde_json::to_value(session).unwrap();
        let terminal = serde_json::to_value(terminal).unwrap();
        assert_eq!(session["opaqueSession"], "opaque-1");
        assert_eq!(terminal["failed"]["message"], "stopped");
        assert!(session.get("endpoint").is_none());
        assert!(session.get("credential").is_none());
    }
}
