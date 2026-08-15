//! Focused transport test for the additive non-content conversation timeline.

use gent_protocol::{
    CONVERSATION_TIMELINE_CAPABILITY, ConversationTimelineFrame, DecisionRecoveryEvidence,
    DecisionSubmission, DependencyActionRequest, DependencyActionResult, DependencyPlan,
    DependencyPlanRequest, Hello, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, Command, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventResume, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt,
};
use tokio::io::duplex;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

#[derive(Clone, Debug)]
struct TimelineRuntime;

impl RuntimeApi for TimelineRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        Ok(CapabilitySet(vec![CONVERSATION_TIMELINE_CAPABILITY.into()]))
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
        unreachable!("not used")
    }

    fn dependency_action(
        &self,
        _: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String> {
        unreachable!("not used")
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

    fn resume_public_run(&self, _: PublicRunResumeRequest) -> Result<PublicRunResponse, String> {
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

    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String> {
        Ok(ConversationTimeline {
            conversation_id: conversation_id.into(),
            runs: Vec::new(),
            artifacts: Vec::new(),
        })
    }
}

#[tokio::test]
async fn conversation_timeline_uses_its_own_negotiated_read_only_extension() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, TimelineRuntime));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![CONVERSATION_TIMELINE_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer)
            if answer.capabilities.0 == vec![CONVERSATION_TIMELINE_CAPABILITY]
    ));
    write_json_frame(
        &mut client,
        &ConversationTimelineFrame::TimelineRequest {
            conversation_id: "conversation-1".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, ConversationTimelineFrame>(&mut client)
            .await
            .unwrap(),
        ConversationTimelineFrame::Timeline(timeline) if timeline.conversation_id == "conversation-1"
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
