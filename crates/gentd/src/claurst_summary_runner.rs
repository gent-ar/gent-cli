use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gent_ports::{
    ClaurstDrainRequest, ClaurstFactValue, ClaurstSessionBinding, ClaurstSourceId,
    ClaurstStartRequest, ConversationSummaryRunner, PortError, PrivateClaurstBridge,
};
use gent_types::{AgentChatConversationId, FrozenConversationContext, NormalizedProviderEvent};
use sha2::{Digest, Sha256};

use crate::standalone_claurst_runtime_factory::StandaloneClaurstBridge;

const DRAIN_LIMIT: u16 = 64;
const MAX_DRAINS: usize = 3_000;
const DRAIN_INTERVAL: Duration = Duration::from_millis(20);
const MAX_OUTPUT_BYTES: usize = 16 * 1024;

impl ConversationSummaryRunner for StandaloneClaurstBridge {
    fn run_summary(
        &self,
        provider: &str,
        model_version: &str,
        prompt: &str,
    ) -> Result<String, PortError> {
        if provider != "claurst" || model_version.trim().is_empty() || prompt.trim().is_empty() {
            return Err(PortError::Unavailable(
                "Claurst summary requires the selected local model".into(),
            ));
        }
        let runner = self.clone();
        let model = model_version.to_owned();
        let prompt = prompt.to_owned();
        thread::Builder::new()
            .name("gent-claurst-summary".into())
            .spawn(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|_| PortError::Unavailable("Claurst summary runtime".into()))?
                    .block_on(run_isolated(&runner, &model, &prompt))
            })
            .map_err(|_| PortError::Unavailable("Claurst summary worker".into()))?
            .join()
            .map_err(|_| PortError::Unavailable("Claurst summary worker".into()))?
    }
}

async fn run_isolated(
    runner: &StandaloneClaurstBridge,
    model: &str,
    prompt: &str,
) -> Result<String, PortError> {
    let bridge = runner.summary_bridge(model).await?;
    let mut digest = Sha256::new();
    digest.update(prompt.as_bytes());
    let digest = format!("{digest:x}", digest = digest.finalize());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PortError::Unavailable("Claurst summary clock".into()))?
        .as_nanos();
    let source_id = ClaurstSourceId(format!("summary-{digest}-{nonce}"));
    let binding = bridge
        .start_summary(ClaurstStartRequest {
            run_id: format!("summary-run-{nonce}"),
            source_id: source_id.clone(),
            turn_id: format!("summary-turn-{nonce}"),
            prompt: prompt.to_owned(),
            context: FrozenConversationContext::cleared(AgentChatConversationId(format!(
                "summary-conversation-{nonce}"
            ))),
            attachments: Vec::new(),
            goal: None,
        })
        .await?;
    drain_summary(&bridge, binding, source_id, prompt).await
}

async fn drain_summary<B: PrivateClaurstBridge>(
    bridge: &B,
    binding: ClaurstSessionBinding,
    source_id: ClaurstSourceId,
    prompt: &str,
) -> Result<String, PortError> {
    let mut cursor = 0;
    let mut output = String::new();
    for _ in 0..MAX_DRAINS {
        let batch = bridge
            .drain(ClaurstDrainRequest {
                run_id: binding.run_id.clone(),
                source_id: source_id.clone(),
                after_cursor: cursor,
                limit: DRAIN_LIMIT,
            })
            .await?;
        for fact in &batch.facts {
            cursor = cursor.max(fact.cursor);
            if let ClaurstFactValue::Event(NormalizedProviderEvent::Output { text, .. }) =
                &fact.value
            {
                output.push_str(text);
                if output.len() > MAX_OUTPUT_BYTES {
                    return Err(PortError::Unavailable(
                        "isolated Claurst summary exceeded output bound".into(),
                    ));
                }
            }
        }
        if let Some(terminal) = batch.terminal {
            return match terminal {
                gent_ports::ClaurstTerminal::Completed => summary_response(&output, prompt),
                gent_ports::ClaurstTerminal::Interrupted
                | gent_ports::ClaurstTerminal::Failed { .. } => Err(PortError::Unavailable(
                    "isolated Claurst summary did not complete".into(),
                )),
            };
        }
        tokio::time::sleep(DRAIN_INTERVAL).await;
    }
    Err(PortError::Unavailable(
        "isolated Claurst summary exceeded its time bound".into(),
    ))
}

fn summary_response(output: &str, prompt: &str) -> Result<String, PortError> {
    if output.trim().is_empty() {
        return Err(PortError::Unavailable(
            "isolated Claurst summary returned no text".into(),
        ));
    }
    if let Some(value) = output
        .match_indices('{')
        .filter_map(|(index, _)| {
            serde_json::Deserializer::from_str(&output[index..])
                .into_iter::<serde_json::Value>()
                .next()
                .and_then(Result::ok)
        })
        .find(|value| {
            value
                .as_object()
                .is_some_and(|value| value.contains_key("title") || value.contains_key("recap"))
        })
    {
        return serde_json::to_string(&value)
            .map_err(|_| PortError::Unavailable("Claurst summary encoding".into()));
    }
    let title = serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .map(|value| humanize(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| clip(output, 60));
    let value = serde_json::to_string(&title)
        .map_err(|_| PortError::Unavailable("Claurst summary encoding".into()))?;
    if prompt.contains("empty `title`") {
        return Ok(format!("{{\"title\":\"\",\"recap\":{value}}}"));
    }
    Ok(format!("{{\"title\":{value},\"recap\":\"\"}}"))
}

fn humanize(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if index > 0 && character.is_uppercase() {
            output.push(' ');
        }
        output.push(character);
    }
    clip(&output, 60)
}

fn clip(value: &str, maximum: usize) -> String {
    value
        .chars()
        .scan(0, |bytes, character| {
            let next = *bytes + character.len_utf8();
            (next <= maximum).then(|| {
                *bytes = next;
                character
            })
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::summary_response;

    #[test]
    fn preserves_provider_summary_json() {
        assert_eq!(
            summary_response("{\"title\":\"Release check\",\"recap\":\"\"}", "").unwrap(),
            "{\"recap\":\"\",\"title\":\"Release check\"}"
        );
    }

    #[test]
    fn normalizes_hermes_tool_template_output_into_a_title() {
        assert_eq!(
            summary_response("{\"name\":\"ReplyPong\",\"arguments\":{}}", "").unwrap(),
            "{\"title\":\"Reply Pong\",\"recap\":\"\"}"
        );
    }

    #[test]
    fn normalizes_a_tool_template_output_into_a_recap() {
        assert_eq!(
            summary_response(
                "{\"name\":\"ReleaseCheck\",\"arguments\":{}}",
                "Return JSON with an empty `title` string and a concise `recap` string."
            )
            .unwrap(),
            "{\"title\":\"\",\"recap\":\"Release Check\"}"
        );
    }

    #[test]
    fn keeps_the_first_summary_object_when_the_provider_repeats_it() {
        assert_eq!(
            summary_response(
                "{\"title\":\"PONG THREE\",\"recap\":\"\"}{\"title\":\"PONG THREE\",",
                ""
            )
            .unwrap(),
            "{\"recap\":\"\",\"title\":\"PONG THREE\"}"
        );
    }
}
