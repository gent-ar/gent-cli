//! Minimal, public transport arguments for locked Claude and Codex executables.

/// Describes whether a public executable begins a new session or resumes a named one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchIntent {
    Start,
    Resume { session_id: String },
}

/// Returns the minimal, documented arguments required to enter a public transport.
///
/// Claude receives newline-delimited stream JSON on standard input. Codex starts its JSON-RPC
/// app server; the native session identifier remains explicit in [`LaunchIntent`] for the later
/// protocol request instead of becoming an undocumented command-line argument.
///
/// # Errors
/// Returns an error for a private or unknown provider, or an empty Claude resume identity.
pub fn arguments(provider: &str, intent: &LaunchIntent) -> Result<Vec<String>, LaunchSpecError> {
    match (provider, intent) {
        ("claude", LaunchIntent::Start) => Ok(claude_stream_arguments()),
        ("claude", LaunchIntent::Resume { session_id }) if !session_id.is_empty() => {
            let mut arguments = claude_stream_arguments();
            arguments.extend(["--resume".into(), session_id.clone()]);
            Ok(arguments)
        }
        ("claude", LaunchIntent::Resume { .. }) => Err(LaunchSpecError::EmptySessionId),
        ("codex", _) => Ok(vec!["app-server".into()]),
        (provider, _) => Err(LaunchSpecError::UnsupportedProvider(provider.into())),
    }
}

fn claude_stream_arguments() -> Vec<String> {
    [
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--print",
        "--verbose",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum LaunchSpecError {
    #[error("unsupported public provider: {0}")]
    UnsupportedProvider(String),
    #[error("a resumed Claude session requires a non-empty session identifier")]
    EmptySessionId,
}

#[cfg(test)]
mod tests {
    use super::{LaunchIntent, LaunchSpecError, arguments};

    #[test]
    fn claude_uses_stream_json_and_passes_only_a_valid_resume_id() {
        assert_eq!(
            arguments("claude", &LaunchIntent::Start).unwrap(),
            [
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--print",
                "--verbose",
            ]
        );
        assert_eq!(
            arguments(
                "claude",
                &LaunchIntent::Resume {
                    session_id: "session-1".into(),
                }
            )
            .unwrap()
            .last(),
            Some(&"session-1".into())
        );
        assert_eq!(
            arguments(
                "claude",
                &LaunchIntent::Resume {
                    session_id: String::new(),
                }
            ),
            Err(LaunchSpecError::EmptySessionId)
        );
    }

    #[test]
    fn codex_uses_its_app_server_and_private_names_are_rejected() {
        assert_eq!(
            arguments(
                "codex",
                &LaunchIntent::Resume {
                    session_id: "thread-1".into(),
                }
            ),
            Ok(vec!["app-server".into()])
        );
        assert!(matches!(
            arguments("claurst", &LaunchIntent::Start),
            Err(LaunchSpecError::UnsupportedProvider(_))
        ));
    }
}
