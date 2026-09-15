use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame},
};
use gent_types::{NormalizedProviderEvent, TokenUsage};
use serde_json::json;

fn events(provider: PublicProvider, frame: &serde_json::Value) -> Vec<NormalizedProviderEvent> {
    normalize_public_frame(provider, frame)
        .into_iter()
        .filter_map(|fact| match fact {
            PublicWireFact::Event(event) => Some(event),
            _ => None,
        })
        .collect()
}

#[test]
fn claude_context_usage_counts_cached_prompt_tokens() {
    let frame = json!({"type":"stream_event","event":{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":8680,"cache_read_input_tokens":13607,"output_tokens":3}}}});
    assert!(events(PublicProvider::Claude, &frame).contains(
        &NormalizedProviderEvent::ContextUsage {
            used_tokens: 22_297,
            window_tokens: None,
        }
    ));
}

#[test]
fn claude_result_reports_every_billed_token_class_for_the_turn() {
    let frame = json!({"type":"result","is_error":false,"usage":{"input_tokens":18,"cache_creation_input_tokens":8843,"cache_read_input_tokens":35894,"output_tokens":155,"output_tokens_details":{"thinking_tokens":69}}});
    assert!(events(PublicProvider::Claude, &frame).contains(
        &NormalizedProviderEvent::TokenUsage {
            usage: TokenUsage {
                input_tokens: 18,
                output_tokens: 155,
                cache_read_tokens: 35_894,
                cache_creation_tokens: 8_843,
            },
        }
    ));
}

#[test]
fn codex_token_usage_update_reports_the_last_call_and_its_context() {
    let frame = json!({"method":"thread/tokenUsage/updated","params":{"threadId":"t","turnId":"u","tokenUsage":{"total":{"totalTokens":38173,"inputTokens":38128,"cachedInputTokens":31744,"cacheWriteInputTokens":0,"outputTokens":45,"reasoningOutputTokens":0},"last":{"totalTokens":19118,"inputTokens":19113,"cachedInputTokens":18816,"cacheWriteInputTokens":12,"outputTokens":5,"reasoningOutputTokens":0},"modelContextWindow":258400}}});
    let events = events(PublicProvider::Codex, &frame);
    assert_eq!(
        events,
        [
            NormalizedProviderEvent::ContextUsage {
                used_tokens: 19_113,
                window_tokens: Some(258_400),
            },
            NormalizedProviderEvent::TokenUsage {
                usage: TokenUsage {
                    input_tokens: 285,
                    output_tokens: 5,
                    cache_read_tokens: 18_816,
                    cache_creation_tokens: 12,
                },
            },
        ]
    );
}
