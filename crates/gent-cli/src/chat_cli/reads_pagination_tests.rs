use gent_protocol::{
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, AgentChatTranscriptFrame, Hello, Negotiated, WireFrame,
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    PROTOCOL_MAX,
};
use tokio::net::UnixListener;

use super::{transcript, transcript_all};

#[tokio::test]
async fn transcript_all_opens_every_durable_history_page() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        for (expected_after, page) in [
            (None, page(vec![event(1), event(2)], Some(2))),
            (Some(2), page(vec![event(3)], None)),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
                    .await
                    .unwrap(),
                AgentChatTranscriptFrame::PageRequest { after_cursor, limit, .. }
                    if after_cursor == expected_after && limit == 100
            ));
            write_json_frame(&mut stream, &AgentChatTranscriptFrame::Page(page))
                .await
                .unwrap();
        }
    });
    let page = transcript_all(Some(directory.path().into()), true, "c1".into())
        .await
        .unwrap();
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(page.next_after_cursor, None);
}

#[tokio::test]
async fn transcript_accepts_and_resumes_a_multi_page_stream() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        for (expected_after, page) in [
            (Some(1), page(vec![event(2), event(3)], Some(3))),
            (Some(3), page(vec![event(4)], None)),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { .. })
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
                    .await
                    .unwrap(),
                AgentChatTranscriptFrame::PageRequest { after_cursor, .. }
                    if after_cursor == expected_after
            ));
            write_json_frame(&mut stream, &AgentChatTranscriptFrame::Page(page))
                .await
                .unwrap();
        }
    });
    let first = transcript(Some(directory.path().into()), true, "c1".into(), Some(1), 2)
        .await
        .unwrap();
    assert_eq!(first.next_after_cursor, Some(3));
    let second = transcript(
        Some(directory.path().into()),
        true,
        "c1".into(),
        first.next_after_cursor,
        2,
    )
    .await
    .unwrap();
    assert_eq!(second.events[0].cursor, 4);
    assert_eq!(second.next_after_cursor, None);
}

#[tokio::test]
async fn transcript_all_rejects_a_repeated_event_identity_across_pages() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let mut repeated = event(2);
        repeated.event_id = "e1".into();
        for (expected_after, page) in [
            (None, page(vec![event(1)], Some(1))),
            (Some(1), page(vec![repeated], None)),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert!(matches!(
                read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
                    .await
                    .unwrap(),
                AgentChatTranscriptFrame::PageRequest { after_cursor, .. }
                    if after_cursor == expected_after
            ));
            write_json_frame(&mut stream, &AgentChatTranscriptFrame::Page(page))
                .await
                .unwrap();
        }
    });
    let error = transcript_all(Some(directory.path().into()), true, "c1".into())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("duplicate transcript event"));
}

fn page(
    events: Vec<NormalizedTranscriptEvent>,
    next_after_cursor: Option<u64>,
) -> NormalizedTranscriptPage {
    NormalizedTranscriptPage {
        conversation_id: "c1".into(),
        events,
        next_after_cursor,
    }
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("e{cursor}"),
        turn_id: "t1".into(),
        run_id: "r1".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "ok".into(),
        is_partial: false,
    }
}
