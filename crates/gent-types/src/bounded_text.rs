use std::borrow::Cow;

pub const MAX_TRANSCRIPT_TEXT_BYTES: usize = 64 * 1024;
pub const INTERRUPTED_REPLY_EVENT_PREFIX: &str = "interrupted:";
pub const OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC: &str = "oversizedProviderFrame";
pub const OVERSIZED_PROVIDER_FRAME_NOTICE: &str = "A provider message larger than 16 MiB was skipped, so part of this turn (usually a tool output) is missing.";
pub const PROVIDER_CONTEXT_COMPACTED_DIAGNOSTIC: &str = "providerContextCompacted";
pub const PROVIDER_CONTEXT_COMPACTED_NOTICE: &str =
    "Context compacted; the conversation continues from the provider's summary.";
pub const PROVIDER_CONTEXT_COMPACTION_FAILED_DIAGNOSTIC: &str = "providerContextCompactionFailed";
pub const PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE: &str =
    "The provider could not compact this conversation's context.";
pub const PROVIDER_SESSION_RECOVERED_DIAGNOSTIC: &str = "providerSessionRecovered";
pub const PROVIDER_SESSION_RECOVERED_NOTICE: &str =
    "The provider session could not be resumed; continued from Gent's saved history.";
pub const PROVIDER_SESSION_UNAVAILABLE_NOTICE: &str = "The provider no longer has this conversation's session. Continue from Gent's saved history to keep going.";

#[must_use]
pub fn bounded_text(text: &str, limit: usize) -> Cow<'_, str> {
    if text.len() <= limit {
        return Cow::Borrowed(text);
    }
    let marker =
        |omitted: usize| format!("\n[… {omitted} bytes omitted by Gent to fit its limit …]\n");
    let kept = limit.saturating_sub(marker(text.len()).len());
    let head = floor_boundary(text, kept / 2);
    let tail = ceil_boundary(text, text.len() - (kept - kept / 2));
    let omitted = tail - head;
    Cow::Owned(format!(
        "{}{}{}",
        &text[..head],
        marker(omitted),
        &text[tail..]
    ))
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::bounded_text;

    #[test]
    fn short_text_is_unchanged() {
        assert_eq!(bounded_text("abc", 3), "abc");
    }

    #[test]
    fn long_text_keeps_its_head_and_tail_with_an_explicit_marker_within_the_limit() {
        let text = format!("FIRST{}LAST", "é".repeat(50_000));
        let bounded = bounded_text(&text, 4_096);
        assert!(bounded.len() <= 4_096);
        assert!(bounded.starts_with("FIRST"));
        assert!(bounded.ends_with("LAST"));
        assert!(bounded.contains("bytes omitted by Gent to fit its limit"));
    }
}
