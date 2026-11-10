use gent_types::{NormalizedProviderEvent, ProviderFailureClassification};
use serde_json::Value;

use super::PublicWireFact;

const SIGN_IN_REQUIRED: &str = "Claude sign-in expired. Sign in again to continue.";

pub(crate) fn authentication_failed(frame: &Value) -> bool {
    frame.get("error").and_then(Value::as_str) == Some("authentication_failed")
        || frame.pointer("/error/type").and_then(Value::as_str) == Some("authentication_error")
}

pub(crate) fn sign_in_required(fact: PublicWireFact) -> PublicWireFact {
    match fact {
        PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { .. }) => {
            PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::Authentication,
                message: SIGN_IN_REQUIRED.into(),
            })
        }
        fact => fact,
    }
}

pub(crate) fn failure(frame: &Value) -> NormalizedProviderEvent {
    let text = [
        frame.get("message").and_then(Value::as_str),
        frame.pointer("/error/message").and_then(Value::as_str),
        frame.get("result").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    let (classification, message) = if authentication_failed(frame) {
        (
            ProviderFailureClassification::Authentication,
            SIGN_IN_REQUIRED,
        )
    } else if text.contains("rate") || text.contains("limit") {
        (
            ProviderFailureClassification::RateLimited,
            "Claude rate limit reached.",
        )
    } else if text.contains("context") || text.contains("token limit") {
        (
            ProviderFailureClassification::ContextLimit,
            "Claude context limit reached.",
        )
    } else {
        (
            ProviderFailureClassification::Provider,
            "Claude ended the turn with an error.",
        )
    };
    NormalizedProviderEvent::ProviderFailure {
        classification,
        message: message.into(),
    }
}
