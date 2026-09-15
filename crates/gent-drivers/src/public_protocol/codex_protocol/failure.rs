use gent_types::ProviderFailureClassification;
use serde_json::Value;

pub(super) fn classify(
    error: Option<&Value>,
    fallback: &'static str,
) -> (ProviderFailureClassification, &'static str) {
    let typed = error
        .and_then(|error| error.get("codexErrorInfo"))
        .and_then(Value::as_str);
    match typed {
        Some("unauthorized") => (
            ProviderFailureClassification::Authentication,
            "Codex authentication failed.",
        ),
        Some("usageLimitExceeded") => (
            ProviderFailureClassification::RateLimited,
            "Codex usage limit reached.",
        ),
        Some("rateLimitExceeded" | "serverOverloaded") => (
            ProviderFailureClassification::RateLimited,
            "Codex rate limit reached.",
        ),
        Some("contextWindowExceeded") => (
            ProviderFailureClassification::ContextLimit,
            "Codex context limit reached.",
        ),
        _ => from_message(error, fallback),
    }
}

fn from_message(
    error: Option<&Value>,
    fallback: &'static str,
) -> (ProviderFailureClassification, &'static str) {
    let text = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if text.contains("401") || text.contains("unauthor") || text.contains("missing bearer") {
        (
            ProviderFailureClassification::Authentication,
            "Codex authentication failed.",
        )
    } else if text.contains("rate limit") {
        (
            ProviderFailureClassification::RateLimited,
            "Codex rate limit reached.",
        )
    } else if text.contains("context") || text.contains("token limit") {
        (
            ProviderFailureClassification::ContextLimit,
            "Codex context limit reached.",
        )
    } else if text.contains("model")
        && (text.contains("not supported")
            || text.contains("not available")
            || text.contains("not found"))
    {
        (
            ProviderFailureClassification::Provider,
            "The selected Codex model is unavailable.",
        )
    } else {
        (ProviderFailureClassification::Provider, fallback)
    }
}
