use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

impl TokenUsage {
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens)
    }

    #[must_use]
    pub const fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::TokenUsage;

    #[test]
    fn every_billed_token_class_counts_toward_the_total() {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 155,
            cache_read_tokens: 35_894,
            cache_creation_tokens: 8_843,
        };
        assert_eq!(usage.total(), 44_902);
        assert_eq!(usage.prompt_tokens(), 44_747);
        assert_eq!(
            serde_json::to_value(usage).unwrap(),
            serde_json::json!({"inputTokens": 10, "outputTokens": 155, "cacheReadTokens": 35_894, "cacheCreationTokens": 8_843})
        );
    }
}
