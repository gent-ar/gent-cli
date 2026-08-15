//! Dedicated frames for an app-owned external-provider bridge.

use gent_types::{
    CapabilitySet, Command, DecisionCommand, ExternalProviderSession, ExternalProviderTerminal,
    ProviderEvent,
};
use serde::{Deserialize, Serialize};

/// Capability required before a private bridge accepts lifecycle requests.
pub const EXTERNAL_PROVIDER_BRIDGE_CAPABILITY: &str = "external-provider-bridge-v1";

/// Version range and capabilities offered by a private bridge endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderBridgeHello {
    pub protocol_min: u16,
    pub protocol_max: u16,
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

/// Version and capabilities accepted by a private bridge endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderBridgeNegotiated {
    pub protocol: u16,
    pub capabilities: CapabilitySet,
}

/// Frames for a private bridge endpoint, never the public Gent client endpoint.
///
/// The contract intentionally carries no provider endpoint, credential, or installation detail.
/// `opaque_session` values are meaningful only to the app-owned bridge implementation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ExternalProviderBridgeFrame {
    Hello(ExternalProviderBridgeHello),
    Negotiated(ExternalProviderBridgeNegotiated),
    CapabilityRegistration,
    Capabilities(CapabilitySet),
    StartRun {
        run_id: String,
    },
    ResumeRun {
        run_id: String,
    },
    Session(ExternalProviderSession),
    Submit {
        opaque_session: String,
        command: Command,
    },
    AnswerDecision {
        opaque_session: String,
        decision: DecisionCommand,
    },
    Interrupt {
        opaque_session: String,
    },
    NextEvent {
        opaque_session: String,
    },
    Event(Option<ProviderEvent>),
    TerminalState {
        opaque_session: String,
    },
    Terminal(Option<ExternalProviderTerminal>),
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        EXTERNAL_PROVIDER_BRIDGE_CAPABILITY, ExternalProviderBridgeFrame,
        ExternalProviderBridgeHello,
    };
    use gent_types::CapabilitySet;

    #[test]
    fn bridge_handshake_and_lifecycle_frames_are_explicit() {
        let hello = ExternalProviderBridgeFrame::Hello(ExternalProviderBridgeHello {
            protocol_min: 1,
            protocol_max: 1,
            capabilities: CapabilitySet(vec![EXTERNAL_PROVIDER_BRIDGE_CAPABILITY.into()]),
        });
        let start = ExternalProviderBridgeFrame::StartRun {
            run_id: "run-1".into(),
        };

        let hello = serde_json::to_value(hello).unwrap();
        let start = serde_json::to_value(start).unwrap();
        assert_eq!(hello["type"], "hello");
        assert_eq!(start["type"], "startRun");
        assert_eq!(start["body"]["run_id"], "run-1");
    }

    #[test]
    fn wire_contract_excludes_private_provider_configuration() {
        let frame = ExternalProviderBridgeFrame::CapabilityRegistration;
        let value = serde_json::to_value(frame).unwrap();
        let text = value.to_string();

        assert_eq!(value["type"], "capabilityRegistration");
        assert!(!text.contains("endpoint"));
        assert!(!text.contains("credential"));
    }
}
