use gent_types::ProviderFailureClassification;
use serde_json::Value;

const SIGN_IN_EXPIRED: (ProviderFailureClassification, &str) = (
    ProviderFailureClassification::Authentication,
    "Codex sign-in expired. Sign in again to continue.",
);
const CONNECTION_LOST: (ProviderFailureClassification, &str) = (
    ProviderFailureClassification::Provider,
    "Codex lost its connection to the model service.",
);

pub(super) fn classify(
    error: Option<&Value>,
    fallback: &'static str,
) -> (ProviderFailureClassification, &'static str) {
    match error.and_then(|error| error.get("codexErrorInfo")) {
        Some(Value::String(code)) => named(code, fallback),
        Some(Value::Object(variant)) => variant.iter().next().map_or(
            (ProviderFailureClassification::Provider, fallback),
            |(name, detail)| structured(name, detail.get("httpStatusCode").and_then(Value::as_u64)),
        ),
        _ => (ProviderFailureClassification::Provider, fallback),
    }
}

fn named(code: &str, fallback: &'static str) -> (ProviderFailureClassification, &'static str) {
    use ProviderFailureClassification::{ContextLimit, Overloaded, Provider, RateLimited};
    match code {
        "unauthorized" => SIGN_IN_EXPIRED,
        "usageLimitExceeded" => (RateLimited, "Codex usage limit reached."),
        "rateLimitExceeded" => (RateLimited, "Codex rate limit reached."),
        "serverOverloaded" => (Overloaded, "Codex servers are overloaded."),
        "contextWindowExceeded" => (ContextLimit, "Codex context limit reached."),
        "sessionBudgetExceeded" => (Provider, "Codex reached this session's budget."),
        "badRequest" => (Provider, "Codex rejected the request."),
        "cyberPolicy" | "misalignmentPolicyViolation" => (
            Provider,
            "Codex declined this request under its usage policy.",
        ),
        "internalServerError" => (Provider, "Codex reported an internal server error."),
        "sandboxError" => (Provider, "Codex could not run the command in its sandbox."),
        "threadRollbackFailed" => (Provider, "Codex could not roll back the thread."),
        _ => (Provider, fallback),
    }
}

fn structured(name: &str, status: Option<u64>) -> (ProviderFailureClassification, &'static str) {
    match (name, status) {
        (_, Some(401)) => SIGN_IN_EXPIRED,
        (_, Some(429)) => (
            ProviderFailureClassification::RateLimited,
            "Codex rate limit reached.",
        ),
        ("activeTurnNotSteerable", _) => (
            ProviderFailureClassification::Provider,
            "Codex could not accept input for this turn.",
        ),
        _ => CONNECTION_LOST,
    }
}
