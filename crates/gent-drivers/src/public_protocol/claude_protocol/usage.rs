use gent_types::NormalizedProviderEvent;
use serde_json::Value;

pub(crate) fn context_usage(event: &Value) -> Option<NormalizedProviderEvent> {
    let usage = event.pointer("/message/usage")?;
    let used_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("inputTokens"))
        .and_then(Value::as_u64)?;
    let window_tokens = usage
        .get("context_window")
        .or_else(|| usage.get("contextWindow"))
        .and_then(Value::as_u64);
    Some(NormalizedProviderEvent::ContextUsage {
        used_tokens,
        window_tokens,
    })
}
