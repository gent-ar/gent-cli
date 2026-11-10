use gent_types::ProviderFailureClassification;
use serde_json::Value;

pub(super) fn classify(
    error: Option<&Value>,
    fallback: &'static str,
) -> (ProviderFailureClassification, &'static str) {
    let typed = error.and_then(|error| error.get("codexErrorInfo"));
    let unauthorized = typed
        .is_some_and(|info| info.as_str() == Some("unauthorized") || rejected_credentials(info));
    if unauthorized {
        return (
            ProviderFailureClassification::Authentication,
            "Codex sign-in expired. Sign in again to continue.",
        );
    }
    match typed.and_then(Value::as_str) {
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

fn rejected_credentials(info: &Value) -> bool {
    info.as_object().is_some_and(|variant| {
        variant
            .values()
            .any(|detail| detail.get("httpStatusCode").and_then(Value::as_u64) == Some(401))
    })
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
    if text.contains("rate limit") {
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
