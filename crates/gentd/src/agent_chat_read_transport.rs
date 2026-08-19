//! Capability-gated transport for public agent-chat metadata and transcript reads.

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatTranscriptFrame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches one read-only agent-chat frame after capability negotiation.
pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if supports(capabilities, AGENT_CHAT_CONVERSATIONS_CAPABILITY) {
        if let Ok(frame) = serde_json::from_value::<AgentChatConversationFrame>(raw.clone()) {
            return conversation(stream, runtime, frame).await;
        }
    }
    if supports(capabilities, AGENT_CHAT_TRANSCRIPT_CAPABILITY) {
        if let Ok(frame) = serde_json::from_value::<AgentChatTranscriptFrame>(raw.clone()) {
            return transcript(stream, runtime, frame).await;
        }
    }
    Ok(false)
}

async fn conversation<S, R>(
    stream: &mut S,
    runtime: &R,
    frame: AgentChatConversationFrame,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !matches!(
        frame,
        AgentChatConversationFrame::SummaryRequest { .. }
            | AgentChatConversationFrame::DetailRequest { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatRead",
            "agent-chat conversation response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_conversation(frame) {
        Ok(
            reply
            @ (AgentChatConversationFrame::Summary(_) | AgentChatConversationFrame::Detail(_)),
        ) => write_json_frame(stream, &reply).await?,
        Ok(_) => {
            write_error(
                stream,
                "invalidAgentChatRead",
                "agent-chat runtime returned a request frame",
            )
            .await?;
        }
        Err(message) => write_error(stream, "agentChatReadUnavailable", &message).await?,
    }
    Ok(true)
}

async fn transcript<S, R>(
    stream: &mut S,
    runtime: &R,
    frame: AgentChatTranscriptFrame,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !matches!(frame, AgentChatTranscriptFrame::PageRequest { .. }) {
        write_error(
            stream,
            "invalidAgentChatRead",
            "agent-chat transcript response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_transcript(frame) {
        Ok(AgentChatTranscriptFrame::Page(page)) => {
            write_json_frame(stream, &AgentChatTranscriptFrame::Page(page)).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidAgentChatRead",
                "agent-chat runtime returned a request frame",
            )
            .await?;
        }
        Err(message) => write_error(stream, "agentChatReadUnavailable", &message).await?,
    }
    Ok(true)
}

fn supports(capabilities: &CapabilitySet, capability: &str) -> bool {
    capabilities.0.iter().any(|item| item == capability)
}

#[cfg(test)]
mod tests {
    use gent_protocol::{
        AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
        AgentChatConversationFrame, AgentChatTranscriptFrame, WireFrame, read_frame,
    };
    use gent_types::{CapabilitySet, DoctorReport, EventPage, HostStatus, Receipt};
    use serde_json::json;
    use tokio::io::duplex;

    use super::dispatch;
    use crate::{api::RuntimeApi, transport::observed_capabilities};

    #[derive(Clone)]
    struct Observer;
    impl RuntimeApi for Observer {
        fn capabilities(&self) -> Result<CapabilitySet, String> {
            Ok(CapabilitySet::default())
        }
        fn status(&self) -> Result<HostStatus, String> {
            Err("unused".into())
        }
        fn submit(&self, _: gent_types::Command) -> Result<Receipt, String> {
            Err("unused".into())
        }
        fn read_event_page(&self, _: u64, _: usize) -> Result<EventPage, String> {
            Err("unused".into())
        }
        fn doctor(&self) -> DoctorReport {
            DoctorReport::empty()
        }
        fn dependency_plan(
            &self,
            _: gent_protocol::DependencyPlanRequest,
        ) -> gent_protocol::DependencyPlan {
            unreachable!()
        }
        fn dependency_action(
            &self,
            _: gent_protocol::DependencyActionRequest,
        ) -> Result<gent_protocol::DependencyActionResult, String> {
            Err("unused".into())
        }
        fn submit_decision(
            &self,
            _: gent_types::DecisionCommand,
        ) -> Result<gent_protocol::DecisionSubmission, String> {
            Err("unused".into())
        }
        fn apply_decision_recovery(
            &self,
            _: String,
            _: gent_protocol::DecisionRecoveryEvidence,
        ) -> Result<gent_types::DecisionSettlement, String> {
            Err("unused".into())
        }
        fn start_public_run(
            &self,
            _: gent_protocol::PublicRunStartRequest,
        ) -> Result<gent_protocol::PublicRunResponse, String> {
            Err("unused".into())
        }
        fn resume_public_run(
            &self,
            _: gent_protocol::PublicRunResumeRequest,
        ) -> Result<gent_protocol::PublicRunResponse, String> {
            Err("unused".into())
        }
        fn interrupt_public_run(
            &self,
            _: gent_protocol::PublicRunInterruptRequest,
        ) -> Result<gent_protocol::PublicRunResponse, String> {
            Err("unused".into())
        }
        fn conversation_status(&self, _: &str) -> Result<gent_types::ConversationStatus, String> {
            Err("unused".into())
        }
        fn conversation_timeline(
            &self,
            _: &str,
        ) -> Result<gent_types::ConversationTimeline, String> {
            Err("unused".into())
        }
    }

    #[tokio::test]
    async fn observer_does_not_advertise_or_dispatch_agent_chat_reads() {
        let advertised = observed_capabilities(false, false, false, false);
        assert!(
            !advertised
                .0
                .iter()
                .any(|value| value == AGENT_CHAT_CONVERSATIONS_CAPABILITY)
        );
        assert!(
            !advertised
                .0
                .iter()
                .any(|value| value == AGENT_CHAT_TRANSCRIPT_CAPABILITY)
        );
        let (_, mut writer) = duplex(1024);
        assert!(!dispatch(&mut writer, &Observer, &CapabilitySet::default(), &json!({
            "type": "pageRequest", "body": { "conversationId": "c", "afterCursor": null, "limit": 1 }
        })).await.unwrap());
    }

    #[tokio::test]
    async fn observer_read_is_an_explicit_error_when_a_future_capability_is_injected() {
        let (mut reader, mut writer) = duplex(1024);
        let capabilities = CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]);
        assert!(dispatch(&mut writer, &Observer, &capabilities, &json!({
            "type": "pageRequest", "body": { "conversationId": "c", "afterCursor": null, "limit": 1 }
        })).await.unwrap());
        assert!(
            matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "agentChatReadUnavailable")
        );
    }

    #[test]
    fn request_models_stay_parseable_only_when_their_capability_is_negotiated() {
        assert!(
            serde_json::from_value::<AgentChatConversationFrame>(
                json!({ "type": "summaryRequest", "body": { "conversationId": "c" } })
            )
            .is_ok()
        );
        assert!(serde_json::from_value::<AgentChatTranscriptFrame>(json!({ "type": "pageRequest", "body": { "conversationId": "c", "afterCursor": null, "limit": 1 } })).is_ok());
    }
}
