//! Capability-gated reads of daemon-normalized agent-chat state.

use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatTranscriptFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, NormalizedTranscriptPage,
};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn summary(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<AgentChatConversationSummary, Box<dyn std::error::Error>> {
    let reply = conversation(
        data_dir,
        no_autostart,
        AgentChatConversationFrame::SummaryRequest {
            conversation_id: conversation_id.clone(),
        },
    )
    .await?;
    match reply {
        AgentChatConversationFrame::Summary(value) if value.conversation_id == conversation_id => {
            Ok(value)
        }
        _ => Err("daemon did not return the requested agent-chat summary".into()),
    }
}

pub(crate) async fn detail(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<AgentChatConversationDetail, Box<dyn std::error::Error>> {
    let reply = conversation(
        data_dir,
        no_autostart,
        AgentChatConversationFrame::DetailRequest {
            conversation_id: conversation_id.clone(),
        },
    )
    .await?;
    match reply {
        AgentChatConversationFrame::Detail(value)
            if value.summary.conversation_id == conversation_id =>
        {
            Ok(value)
        }
        _ => Err("daemon did not return the requested agent-chat detail".into()),
    }
}

pub(crate) async fn transcript(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    after_cursor: Option<u64>,
    limit: u16,
) -> Result<NormalizedTranscriptPage, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require(&capabilities.0, AGENT_CHAT_TRANSCRIPT_CAPABILITY)?;
    write_json_frame(
        &mut stream,
        &AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.clone(),
            after_cursor,
            limit,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    let reply = decode::<AgentChatTranscriptFrame>(raw, "agent-chat transcript")?;
    match reply {
        AgentChatTranscriptFrame::Page(page)
            if valid_page(&page, &conversation_id, after_cursor) =>
        {
            Ok(page)
        }
        _ => Err("daemon returned an invalid agent-chat transcript page".into()),
    }
}

async fn conversation(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatConversationFrame,
) -> Result<AgentChatConversationFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require(&capabilities.0, AGENT_CHAT_CONVERSATIONS_CAPABILITY)?;
    write_json_frame(&mut stream, &request).await?;
    decode(
        read_json_frame(&mut stream).await?,
        "agent-chat conversation",
    )
}

fn require(capabilities: &[String], capability: &str) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .iter()
        .any(|item| item == capability)
        .then_some(())
        .ok_or_else(|| "daemon does not support agent-chat reads; upgrade gentd".into())
}

fn decode<T: serde::de::DeserializeOwned>(
    raw: Value,
    label: &str,
) -> Result<T, Box<dyn std::error::Error>> {
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    serde_json::from_value(raw).map_err(|_| format!("daemon did not return {label}").into())
}

fn valid_page(
    page: &NormalizedTranscriptPage,
    conversation_id: &str,
    after_cursor: Option<u64>,
) -> bool {
    if page.conversation_id != conversation_id {
        return false;
    }
    let mut cursor = after_cursor.unwrap_or_default();
    for event in &page.events {
        if event.cursor <= cursor {
            return false;
        }
        cursor = event.cursor;
    }
    page.next_after_cursor.is_none_or(|next| next > cursor)
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Hello, Negotiated, read_frame, write_frame};
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, CapabilitySet,
        NormalizedTranscriptEvent, NormalizedTranscriptKind, PROTOCOL_MAX,
    };
    use tokio::net::UnixListener;

    use super::*;

    #[tokio::test]
    async fn summary_sends_only_a_typed_read_after_negotiation() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(
                matches!(read_frame(&mut stream).await.unwrap(), WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()))
            );
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                read_json_frame::<_, AgentChatConversationFrame>(&mut stream)
                    .await
                    .unwrap(),
                AgentChatConversationFrame::SummaryRequest {
                    conversation_id: "c1".into()
                }
            );
            write_json_frame(
                &mut stream,
                &AgentChatConversationFrame::Summary(test_summary("c1")),
            )
            .await
            .unwrap();
        });
        assert_eq!(
            summary(Some(directory.path().into()), true, "c1".into())
                .await
                .unwrap()
                .conversation_id,
            "c1"
        );
    }

    #[tokio::test]
    async fn transcript_requires_its_capability_without_sending_a_request() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_frame(&mut stream).await.unwrap();
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet::default(),
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            transcript(Some(directory.path().into()), true, "c1".into(), None, 20)
                .await
                .unwrap_err()
                .to_string()
                .contains("upgrade gentd")
        );
    }

    #[tokio::test]
    async fn transcript_rejects_a_nonascending_daemon_page() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
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
            let _ = read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
                .await
                .unwrap();
            write_json_frame(
                &mut stream,
                &AgentChatTranscriptFrame::Page(NormalizedTranscriptPage {
                    conversation_id: "c1".into(),
                    events: vec![event(2), event(2)],
                    next_after_cursor: None,
                }),
            )
            .await
            .unwrap();
        });
        assert!(
            transcript(
                Some(directory.path().into()),
                true,
                "c1".into(),
                Some(1),
                20
            )
            .await
            .is_err()
        );
    }

    fn test_summary(conversation_id: &str) -> AgentChatConversationSummary {
        AgentChatConversationSummary {
            conversation_id: conversation_id.into(),
            title: None,
            updated_at_unix_ms: 1,
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
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
}
