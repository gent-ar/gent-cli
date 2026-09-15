use gent_types::{NormalizedProviderEvent, TokenUsage};
use serde_json::Value;

pub(crate) fn context_usage(event: &Value) -> Option<NormalizedProviderEvent> {
    let usage = token_usage(event.pointer("/message/usage")?)?;
    let window_tokens = event
        .pointer("/message/usage/context_window")
        .and_then(Value::as_u64);
    Some(NormalizedProviderEvent::ContextUsage {
        used_tokens: usage.prompt_tokens(),
        window_tokens,
    })
}

pub(crate) fn turn_usage(frame: &Value) -> Option<NormalizedProviderEvent> {
    token_usage(frame.get("usage")?).map(|usage| NormalizedProviderEvent::TokenUsage { usage })
}

fn token_usage(usage: &Value) -> Option<TokenUsage> {
    let count = |key: &str| usage.get(key).and_then(Value::as_u64);
    Some(TokenUsage {
        input_tokens: count("input_tokens")?,
        output_tokens: count("output_tokens").unwrap_or_default(),
        cache_read_tokens: count("cache_read_input_tokens").unwrap_or_default(),
        cache_creation_tokens: count("cache_creation_input_tokens").unwrap_or_default(),
    })
}
