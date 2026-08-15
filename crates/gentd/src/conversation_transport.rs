//! Capability-gated, content-free conversation read extensions.

use gent_protocol::{
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY, ConversationIndexFrame, ConversationStatusFrame,
    ConversationTimelineFrame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::api::RuntimeApi;
use crate::transport::write_error;

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
    if supports(capabilities, CONVERSATION_INDEX_CAPABILITY)
        && matches!(
            serde_json::from_value(raw.clone()),
            Ok(ConversationIndexFrame::Request)
        )
    {
        return write_index(stream, runtime).await;
    }
    if supports(capabilities, CONVERSATION_STATUS_CAPABILITY) {
        if let Ok(ConversationStatusFrame::Request { conversation_id }) =
            serde_json::from_value(raw.clone())
        {
            return write_status(stream, runtime, &conversation_id).await;
        }
    }
    if supports(capabilities, CONVERSATION_TIMELINE_CAPABILITY) {
        if let Ok(ConversationTimelineFrame::TimelineRequest { conversation_id }) =
            serde_json::from_value(raw.clone())
        {
            return write_timeline(stream, runtime, &conversation_id).await;
        }
    }
    Ok(false)
}

async fn write_index<S, R>(
    stream: &mut S,
    runtime: &R,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    match runtime.conversations() {
        Ok(index) => write_json_frame(stream, &ConversationIndexFrame::Index(index)).await?,
        Err(message) => write_error(stream, "invalidRequest", &message).await?,
    }
    Ok(true)
}

async fn write_status<S, R>(
    stream: &mut S,
    runtime: &R,
    conversation_id: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    match runtime.conversation_status(conversation_id) {
        Ok(status) => write_json_frame(stream, &ConversationStatusFrame::Status(status)).await?,
        Err(message) => write_error(stream, "invalidRequest", &message).await?,
    }
    Ok(true)
}

async fn write_timeline<S, R>(
    stream: &mut S,
    runtime: &R,
    conversation_id: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    match runtime.conversation_timeline(conversation_id) {
        Ok(timeline) => {
            write_json_frame(stream, &ConversationTimelineFrame::Timeline(timeline)).await?;
        }
        Err(message) => write_error(stream, "invalidRequest", &message).await?,
    }
    Ok(true)
}

fn supports(capabilities: &CapabilitySet, capability: &str) -> bool {
    capabilities.0.iter().any(|item| item == capability)
}

#[cfg(test)]
mod tests {
    use gent_protocol::{
        CONVERSATION_INDEX_CAPABILITY, ConversationIndexFrame, DecisionRecoveryEvidence,
        DecisionSubmission, DependencyActionRequest, DependencyActionResult, DependencyPlan,
        DependencyPlanRequest, Hello, PublicRunInterruptRequest, PublicRunResponse,
        PublicRunResumeRequest, PublicRunStartRequest, WireFrame, read_frame, read_json_frame,
        write_frame, write_json_frame,
    };
    use gent_types::{
        CapabilitySet, Command, ConversationListItem, ConversationStatus, ConversationTimeline,
        DecisionCommand, DecisionSettlement, DoctorReport, EventResume, HostStatus, PROTOCOL_MAX,
        PROTOCOL_MIN, Receipt,
    };
    use tokio::io::duplex;

    use crate::api::RuntimeApi;
    use crate::transport::serve_connection;

    #[derive(Clone)]
    struct IndexRuntime {
        unavailable: bool,
    }

    impl RuntimeApi for IndexRuntime {
        fn capabilities(&self) -> Result<CapabilitySet, String> {
            Ok(CapabilitySet(vec![CONVERSATION_INDEX_CAPABILITY.into()]))
        }
        fn status(&self) -> Result<HostStatus, String> {
            Err("not used".into())
        }
        fn submit(&self, _: Command) -> Result<Receipt, String> {
            Err("not used".into())
        }
        fn resume_events(&self, _: u64) -> Result<EventResume, String> {
            Err("not used".into())
        }
        fn doctor(&self) -> DoctorReport {
            DoctorReport::empty()
        }
        fn dependency_plan(&self, _: DependencyPlanRequest) -> DependencyPlan {
            unreachable!()
        }
        fn dependency_action(
            &self,
            _: DependencyActionRequest,
        ) -> Result<DependencyActionResult, String> {
            unreachable!()
        }
        fn submit_decision(&self, _: DecisionCommand) -> Result<DecisionSubmission, String> {
            Err("not used".into())
        }
        fn apply_decision_recovery(
            &self,
            _: String,
            _: DecisionRecoveryEvidence,
        ) -> Result<DecisionSettlement, String> {
            Err("not used".into())
        }
        fn start_public_run(&self, _: PublicRunStartRequest) -> Result<PublicRunResponse, String> {
            Err("not used".into())
        }
        fn resume_public_run(
            &self,
            _: PublicRunResumeRequest,
        ) -> Result<PublicRunResponse, String> {
            Err("not used".into())
        }
        fn interrupt_public_run(
            &self,
            _: PublicRunInterruptRequest,
        ) -> Result<PublicRunResponse, String> {
            Err("not used".into())
        }
        fn conversation_status(&self, _: &str) -> Result<ConversationStatus, String> {
            Err("not used".into())
        }
        fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
            if self.unavailable {
                return Err("ledger unavailable".into());
            }
            Ok(vec![ConversationListItem {
                conversation_id: "conversation-1".into(),
                run_count: 1,
            }])
        }
        fn conversation_timeline(&self, _: &str) -> Result<ConversationTimeline, String> {
            Err("not used".into())
        }
    }

    fn hello(capabilities: Vec<&str>) -> WireFrame {
        WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(capabilities.into_iter().map(str::to_owned).collect()),
        })
    }

    #[tokio::test]
    async fn index_is_negotiated_and_returns_only_content_free_items() {
        let (mut client, server) = duplex(1024);
        let task = tokio::spawn(serve_connection(
            server,
            IndexRuntime { unavailable: false },
        ));
        write_frame(&mut client, &hello(vec![CONVERSATION_INDEX_CAPABILITY]))
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Negotiated(answer) if answer.capabilities.0 == vec![CONVERSATION_INDEX_CAPABILITY]
        ));
        write_json_frame(&mut client, &ConversationIndexFrame::Request)
            .await
            .unwrap();
        assert!(matches!(
            read_json_frame::<_, ConversationIndexFrame>(&mut client).await.unwrap(),
            ConversationIndexFrame::Index(items)
                if items == vec![ConversationListItem { conversation_id: "conversation-1".into(), run_count: 1 }]
        ));
        drop(client);
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn index_request_without_negotiated_capability_is_rejected() {
        let (mut client, server) = duplex(1024);
        let task = tokio::spawn(serve_connection(
            server,
            IndexRuntime { unavailable: false },
        ));
        write_frame(&mut client, &hello(Vec::new())).await.unwrap();
        let _ = read_frame(&mut client).await.unwrap();
        write_json_frame(&mut client, &ConversationIndexFrame::Request)
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Error { code, .. } if code == "invalidCommand"
        ));
        drop(client);
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn index_runtime_failure_returns_a_protocol_error_without_side_effects() {
        let (mut client, server) = duplex(1024);
        let task = tokio::spawn(serve_connection(server, IndexRuntime { unavailable: true }));
        write_frame(&mut client, &hello(vec![CONVERSATION_INDEX_CAPABILITY]))
            .await
            .unwrap();
        let _ = read_frame(&mut client).await.unwrap();
        write_json_frame(&mut client, &ConversationIndexFrame::Request)
            .await
            .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Error { code, message } if code == "invalidRequest" && message == "ledger unavailable"
        ));
        drop(client);
        assert!(task.await.unwrap().is_err());
    }
}
