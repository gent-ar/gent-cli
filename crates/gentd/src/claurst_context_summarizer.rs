use std::time::Duration;

use async_trait::async_trait;
use gent_drivers::conversation_context_summary::{SummaryRequest, summary_text};
use gent_types::ContextCompactionFailure;
use serde_json::{Value, json};

const SUMMARY_MAX_TOKENS: u32 = 1_024;
const SUMMARY_PROMPT_RESERVE_TOKENS: u32 = 512;
const BYTES_PER_TOKEN: usize = 3;
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalContextWindow {
    pub(crate) history_input_bytes: usize,
    pub(crate) summary_input_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextSummary {
    pub(crate) text: String,
    pub(crate) tokens: u32,
}

#[async_trait]
pub(crate) trait ContextSummarizer: Send + Sync + std::fmt::Debug {
    fn window(&self) -> LocalContextWindow;

    async fn summarize(
        &self,
        request: SummaryRequest,
    ) -> Result<ContextSummary, ContextCompactionFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LlamaSummaryEndpoint {
    pub(crate) server_url: String,
    pub(crate) model: String,
    pub(crate) context_tokens: u32,
    pub(crate) history_input_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LlamaContextSummarizer {
    client: reqwest::Client,
    endpoint: LlamaSummaryEndpoint,
}

impl LlamaContextSummarizer {
    #[must_use]
    pub(crate) fn new(endpoint: LlamaSummaryEndpoint) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
        }
    }
}

#[async_trait]
impl ContextSummarizer for LlamaContextSummarizer {
    fn window(&self) -> LocalContextWindow {
        let tokens = self
            .endpoint
            .context_tokens
            .saturating_sub(SUMMARY_MAX_TOKENS + SUMMARY_PROMPT_RESERVE_TOKENS);
        LocalContextWindow {
            history_input_bytes: self.endpoint.history_input_bytes,
            summary_input_bytes: usize::try_from(tokens)
                .unwrap_or(usize::MAX)
                .saturating_mul(BYTES_PER_TOKEN),
        }
    }

    async fn summarize(
        &self,
        request: SummaryRequest,
    ) -> Result<ContextSummary, ContextCompactionFailure> {
        let body = serde_json::to_vec(&completion_body(&self.endpoint.model, &request))
            .map_err(|_| ContextCompactionFailure::RuntimeUnavailable)?;
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.endpoint.server_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(SUMMARY_TIMEOUT)
            .body(body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| ContextCompactionFailure::RuntimeUnavailable)?;
        let bytes = response
            .bytes()
            .await
            .map_err(|_| ContextCompactionFailure::RuntimeUnavailable)?;
        let value = serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| ContextCompactionFailure::RuntimeUnavailable)?;
        completion_summary(&value)
    }
}

pub(crate) fn completion_body(model: &str, request: &SummaryRequest) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": request.instructions},
            {"role": "user", "content": request.content},
        ],
        "max_tokens": SUMMARY_MAX_TOKENS,
        "temperature": 0.2,
        "stream": false,
        "chat_template_kwargs": {"gent_instructions": "", "enable_thinking": false},
    })
}

pub(crate) fn completion_summary(
    value: &Value,
) -> Result<ContextSummary, ContextCompactionFailure> {
    let choice = value
        .pointer("/choices/0")
        .ok_or(ContextCompactionFailure::RuntimeUnavailable)?;
    if choice.get("finish_reason").and_then(Value::as_str) != Some("stop") {
        return Err(ContextCompactionFailure::OutputLimit);
    }
    let text = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .and_then(summary_text)
        .ok_or(ContextCompactionFailure::EmptySummary)?;
    Ok(ContextSummary {
        text,
        tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
#[path = "claurst_context_summarizer_tests.rs"]
mod tests;
