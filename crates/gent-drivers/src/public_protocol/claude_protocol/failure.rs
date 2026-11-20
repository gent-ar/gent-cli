use gent_types::{NormalizedProviderEvent, ProviderFailureClassification};
use serde_json::Value;

use super::PublicWireFact;

const SIGN_IN_REQUIRED: &str = "Claude sign-in expired. Sign in again to continue.";
const RATE_LIMITED: &str = "Claude rate limit reached.";

pub(crate) fn authentication_failed(frame: &Value) -> bool {
    frame.get("error").and_then(Value::as_str) == Some("authentication_failed")
        || frame.pointer("/error/type").and_then(Value::as_str) == Some("authentication_error")
}

pub(crate) fn sign_in_required(fact: PublicWireFact) -> PublicWireFact {
    reclassified(
        fact,
        ProviderFailureClassification::Authentication,
        SIGN_IN_REQUIRED,
        None,
    )
}

pub(crate) fn rate_limit_reached(fact: PublicWireFact, resets_at: Option<u64>) -> PublicWireFact {
    reclassified(
        fact,
        ProviderFailureClassification::RateLimited,
        RATE_LIMITED,
        resets_at,
    )
}

pub(crate) fn rate_limit_resets_at(frame: &Value) -> Option<u64> {
    (frame.get("type").and_then(Value::as_str) == Some("rate_limit_event"))
        .then(|| frame.pointer("/rate_limit_info/resetsAt"))
        .flatten()
        .and_then(Value::as_u64)
}

fn reclassified(
    fact: PublicWireFact,
    classification: ProviderFailureClassification,
    message: &str,
    resets_at: Option<u64>,
) -> PublicWireFact {
    match fact {
        PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure { .. }) => {
            PublicWireFact::Event(NormalizedProviderEvent::ProviderFailure {
                classification,
                message: message.into(),
                resets_at,
            })
        }
        fact => fact,
    }
}

pub(crate) fn rate_limited(frame: &Value) -> bool {
    frame.get("error").and_then(Value::as_str) == Some("rate_limit")
        || frame.pointer("/error/type").and_then(Value::as_str) == Some("rate_limit_error")
        || frame.get("api_error_status").and_then(Value::as_u64) == Some(429)
        || frame
            .pointer("/rate_limit_info/status")
            .and_then(Value::as_str)
            == Some("rejected")
}

pub(crate) fn failure(frame: &Value) -> NormalizedProviderEvent {
    let (classification, message) = if authentication_failed(frame) {
        (
            ProviderFailureClassification::Authentication,
            SIGN_IN_REQUIRED,
        )
    } else if rate_limited(frame) {
        (ProviderFailureClassification::RateLimited, RATE_LIMITED)
    } else if frame.get("terminal_reason").and_then(Value::as_str) == Some("prompt_too_long") {
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
        resets_at: None,
    }
}
