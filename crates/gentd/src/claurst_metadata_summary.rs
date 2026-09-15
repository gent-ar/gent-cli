use gent_ports::{ConversationSummaryRunner, PortError};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClaurstMetadataSummaryRunner;

impl ConversationSummaryRunner for ClaurstMetadataSummaryRunner {
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
        let transcript = prompt.split_once("\n\n").map_or(prompt, |(_, value)| value);
        let lines = transcript
            .lines()
            .filter_map(|line| line.split_once(": ").map(|(_, value)| value.trim()))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let text = if prompt.contains("empty `title`") {
            lines
                .iter()
                .rev()
                .take(3)
                .rev()
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            lines.first().copied().unwrap_or_default().to_owned()
        };
        if text.is_empty() {
            return Err(PortError::Unavailable(
                "Claurst summary transcript is empty".into(),
            ));
        }
        let value = serde_json::to_string(&text)
            .map_err(|_| PortError::Unavailable("Claurst summary encoding".into()))?;
        if prompt.contains("empty `title`") {
            Ok(format!("{{\"title\":\"\",\"recap\":{value}}}"))
        } else {
            Ok(format!("{{\"title\":{value},\"recap\":\"\"}}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::ConversationSummaryRunner;

    use super::ClaurstMetadataSummaryRunner;

    #[test]
    fn derives_title_without_starting_another_model_turn() {
        let response = ClaurstMetadataSummaryRunner
            .run_summary(
                "claurst",
                "qwen",
                "Return JSON with one short `title` string and an empty `recap` string.\n\nuser: Fix silent prompts\nassistant: Done",
            )
            .unwrap();
        assert_eq!(
            response,
            "{\"title\":\"Fix silent prompts\",\"recap\":\"\"}"
        );
    }

    #[test]
    fn derives_recap_from_the_latest_conversation_lines() {
        let response = ClaurstMetadataSummaryRunner
            .run_summary(
                "claurst",
                "qwen",
                "Return JSON with an empty `title` string and a concise `recap` string.\n\nuser: one\nassistant: two\nuser: three\nassistant: four",
            )
            .unwrap();
        assert_eq!(response, "{\"title\":\"\",\"recap\":\"two three four\"}");
    }
}
