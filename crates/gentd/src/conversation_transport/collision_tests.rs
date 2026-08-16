//! Regression coverage for extension frames which share the legacy `request` tag.

use gent_protocol::{
    CONVERSATION_CONTENT_CAPABILITY, CONVERSATION_INDEX_CAPABILITY, ConversationContentFrame,
    DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest, Hello, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, Command, ConversationContentPage, ConversationListItem, ConversationStatus,
    ConversationTimeline, DecisionCommand, DecisionSettlement, DoctorReport, EventResume,
    HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt,
};
use tokio::io::duplex;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

#[derive(Clone)]
struct ContentRuntime;

impl RuntimeApi for ContentRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        Ok(CapabilitySet(vec![
            CONVERSATION_INDEX_CAPABILITY.into(),
            CONVERSATION_CONTENT_CAPABILITY.into(),
        ]))
    }
    fn status(&self) -> Result<HostStatus, String> {
        Err("unused".into())
    }
    fn submit(&self, _: Command) -> Result<Receipt, String> {
        Err("unused".into())
    }
    fn resume_events(&self, _: u64) -> Result<EventResume, String> {
        Err("unused".into())
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
        Err("unused".into())
    }
    fn apply_decision_recovery(
        &self,
        _: String,
        _: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String> {
        Err("unused".into())
    }
    fn start_public_run(&self, _: PublicRunStartRequest) -> Result<PublicRunResponse, String> {
        Err("unused".into())
    }
    fn resume_public_run(&self, _: PublicRunResumeRequest) -> Result<PublicRunResponse, String> {
        Err("unused".into())
    }
    fn interrupt_public_run(
        &self,
        _: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String> {
        Err("unused".into())
    }
    fn conversation_status(&self, _: &str) -> Result<ConversationStatus, String> {
        Err("unused".into())
    }
    fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
        Ok(Vec::new())
    }
    fn conversation_timeline(&self, _: &str) -> Result<ConversationTimeline, String> {
        Err("unused".into())
    }
    fn conversation_content(
        &self,
        id: &str,
        _: Option<gent_types::ConversationContentCursor>,
        _: u16,
    ) -> Result<ConversationContentPage, String> {
        Ok(ConversationContentPage {
            conversation_id: id.into(),
            entries: Vec::new(),
            next_before: None,
        })
    }
}

#[tokio::test]
async fn structured_content_request_wins_before_the_bodyless_index_request() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, ContentRuntime));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                CONVERSATION_INDEX_CAPABILITY.into(),
                CONVERSATION_CONTENT_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_json_frame(
        &mut client,
        &ConversationContentFrame::Request {
            conversation_id: "conversation-1".into(),
            before: None,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, ConversationContentFrame>(&mut client).await.unwrap(),
        ConversationContentFrame::Page(page) if page.conversation_id == "conversation-1"
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
