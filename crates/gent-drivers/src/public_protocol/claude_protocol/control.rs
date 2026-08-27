use gent_types::NormalizedProviderEvent;
use serde_json::Value;

use super::super::PublicWireFact;

pub(crate) fn control_response(frame: &Value) -> Vec<PublicWireFact> {
    match frame.pointer("/response/subtype").and_then(Value::as_str) {
        Some("success") => Vec::new(),
        Some("error") => diagnostic("claudeControlResponseError"),
        Some(_) => diagnostic("unsupportedClaudeControlResponse"),
        None => diagnostic("malformedClaudeControlResponse"),
    }
}

fn diagnostic(classification: &str) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    )]
}
