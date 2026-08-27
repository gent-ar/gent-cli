use super::*;

fn event(kind: NormalizedTranscriptKind, text: &str) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor: 1,
        event_id: "event-1".into(),
        turn_id: "turn-1".into(),
        run_id: "run-1".into(),
        kind,
        text: text.into(),
        is_partial: false,
    }
}

#[test]
fn cadence_matches_native_summary_schedule() {
    assert!(summary_due(ConversationSummaryKind::Title, 1));
    assert!(summary_due(ConversationSummaryKind::Title, 2));
    assert!(summary_due(ConversationSummaryKind::Recap, 6));
    assert!(summary_due(ConversationSummaryKind::Recap, 12));
    assert!(!summary_due(ConversationSummaryKind::Recap, 7));
}

#[test]
fn scheduler_retries_missing_titles_and_creates_recaps_at_native_cadence() {
    let events = (1..=6)
        .map(|turn| NormalizedTranscriptEvent {
            cursor: turn,
            event_id: format!("event-{turn}"),
            turn_id: format!("turn-{turn}"),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "done".into(),
            is_partial: false,
        })
        .collect::<Vec<_>>();
    let first = scheduled_requests("conversation", "claude", "haiku", &events[..1], &[]).unwrap();
    assert_eq!(
        first.iter().map(|request| request.kind).collect::<Vec<_>>(),
        vec![ConversationSummaryKind::Title]
    );
    let retry = scheduled_requests("conversation", "claude", "haiku", &events[..2], &[]).unwrap();
    assert_eq!(
        retry.iter().map(|request| request.kind).collect::<Vec<_>>(),
        vec![ConversationSummaryKind::Title]
    );
    let recap = scheduled_requests("conversation", "claude", "haiku", &events, &[]).unwrap();
    assert_eq!(
        recap.iter().map(|request| request.kind).collect::<Vec<_>>(),
        vec![
            ConversationSummaryKind::Title,
            ConversationSummaryKind::Recap
        ]
    );
}

#[test]
fn request_has_stable_digest_and_excludes_partial_thinking() {
    let events = vec![
        event(NormalizedTranscriptKind::UserMessage, "build it"),
        event(NormalizedTranscriptKind::Thinking, "private"),
        NormalizedTranscriptEvent {
            is_partial: true,
            ..event(NormalizedTranscriptKind::AssistantMessage, "partial")
        },
        event(NormalizedTranscriptKind::AssistantMessage, "done"),
    ];
    let result = request(
        "conversation-1",
        ConversationSummaryKind::Title,
        vec!["turn-1".into()],
        "claude",
        "haiku",
        &events,
    )
    .unwrap();
    assert!(result.prompt.contains("user: build it"));
    assert!(result.prompt.contains("assistant: done"));
    assert!(!result.prompt.contains("private"));
    assert!(!result.prompt.contains("partial"));
    assert_eq!(result.input_digest.len(), 64);
}

#[test]
fn request_compacts_unicode_transcript_without_splitting_characters() {
    let text = "é".repeat(20_000);
    let request = request(
        "conversation-1",
        ConversationSummaryKind::Title,
        vec!["turn-1".into()],
        "claude",
        "haiku",
        &[event(NormalizedTranscriptKind::UserMessage, &text)],
    )
    .unwrap();
    assert!(
        request
            .prompt
            .contains("Earlier conversation content omitted")
    );
    assert!(request.prompt.is_char_boundary(request.prompt.len()));
}

#[test]
fn complete_persists_provenance_and_selected_field() {
    let request = request(
        "conversation-1",
        ConversationSummaryKind::Title,
        vec!["turn-1".into()],
        "codex",
        "gpt-5",
        &[event(NormalizedTranscriptKind::UserMessage, "hello")],
    )
    .unwrap();
    let artifact = complete(
        &request,
        "artifact-1".into(),
        r#"{"title":"A useful title","recap":"ignored"}"#,
        None,
    )
    .unwrap();
    assert_eq!(artifact.text.as_deref(), Some("A useful title"));
    assert_eq!(artifact.provider, "codex");
    assert_eq!(artifact.status, ConversationArtifactStatus::Completed);
}

#[test]
fn malformed_output_is_rejected_and_fenced_metadata_is_clipped() {
    let request = request(
        "conversation-1",
        ConversationSummaryKind::Recap,
        vec!["turn-1".into()],
        "claude",
        "haiku",
        &[event(NormalizedTranscriptKind::UserMessage, "hello")],
    )
    .unwrap();
    assert_eq!(
        complete(&request, "artifact-1".into(), "not json", None),
        Err(ConversationSummaryError::InvalidResponse)
    );
    let response = format!(
        "```json\n{{\"title\":\"\",\"recap\":\"{}\"}}\n```",
        "x ".repeat(3_000)
    );
    assert_eq!(
        complete(&request, "artifact-2".into(), &response, None)
            .unwrap()
            .text
            .unwrap()
            .len(),
        MAX_RECAP_BYTES - 1
    );
}
