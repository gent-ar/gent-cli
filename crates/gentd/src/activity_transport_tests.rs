//! Transport checks for the reserved, authority-gated activity endpoint.

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame, DecisionRecoveryEvidence,
    DecisionSubmission, DependencyActionRequest, DependencyActionResult, DependencyPlan,
    DependencyPlanRequest, Hello, PublicRunInterruptRequest, PublicRunResponse,
    PublicRunResumeRequest, PublicRunStartRequest, RUNTIME_UPDATE_CHECK_CAPABILITY, WireFrame,
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_runtime::ConversationActivityRead;
use gent_types::{
    CONVERSATION_ACTIVITY_SCHEMA_VERSION, CapabilitySet, Command, ConversationActivity,
    ConversationActivityState, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventResume, HostEpoch, HostStatus, PROTOCOL_MAX,
    PROTOCOL_MIN, Receipt, RuntimeReleaseChannel, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeVersion, TurnPhase,
};
use tokio::io::duplex;

use crate::{
    api::RuntimeApi,
    transport::{observed_capabilities, serve_connection},
};

#[derive(Clone, Debug)]
struct ActivityRuntime {
    update_checks: bool,
}

impl RuntimeApi for ActivityRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        let mut capabilities = vec![CONVERSATION_ACTIVITY_CAPABILITY.into()];
        if self.update_checks {
            capabilities.push(RUNTIME_UPDATE_CHECK_CAPABILITY.into());
        }
        Ok(CapabilitySet(capabilities))
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
    fn conversation_timeline(&self, _: &str) -> Result<ConversationTimeline, String> {
        Err("not used".into())
    }
    fn runtime_update_check(
        &self,
        request: RuntimeUpdateCheckRequest,
    ) -> Result<RuntimeUpdateCheckReport, String> {
        Ok(RuntimeUpdateCheckReport {
            current_version: RuntimeVersion {
                major: 1,
                minor: 0,
                patch: 0,
            },
            channel: request.channel,
            state: RuntimeUpdateCheckState::Current,
            candidate: None,
            failure: None,
        })
    }
    fn conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        _: u64,
    ) -> Result<ConversationActivityRead, String> {
        Ok(ConversationActivityRead::Snapshot(ConversationActivity {
            schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            host_epoch: HostEpoch(4),
            revision: 2,
            activity_sequence: 3,
            cursor: 8,
            active_turn_id: Some("turn-1".into()),
            root_phase: TurnPhase::Processing,
            state: ConversationActivityState::Thinking,
            pending_decision_id: None,
            work: Vec::new(),
            has_error: false,
        }))
    }
}

#[test]
fn observer_capabilities_do_not_advertise_authority_or_update_work() {
    let capabilities = observed_capabilities(false, false);
    assert!(
        !capabilities
            .0
            .iter()
            .any(|item| item == CONVERSATION_ACTIVITY_CAPABILITY)
    );
    assert!(
        !capabilities
            .0
            .iter()
            .any(|item| item == RUNTIME_UPDATE_CHECK_CAPABILITY)
    );
}

#[tokio::test]
async fn activity_snapshot_requires_a_negotiated_activity_capability() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(
        server,
        ActivityRuntime {
            update_checks: false,
        },
    ));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![CONVERSATION_ACTIVITY_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer)
            if answer.capabilities.0 == vec![CONVERSATION_ACTIVITY_CAPABILITY]
    ));
    write_json_frame(
        &mut client,
        &ConversationActivityFrame::Request {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            after_cursor: 0,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, ConversationActivityFrame>(&mut client)
            .await
            .unwrap(),
        ConversationActivityFrame::Snapshot(activity)
            if activity.cursor == 8 && activity.state == ConversationActivityState::Thinking
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn cached_update_check_requires_its_negotiated_capability() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(
        server,
        ActivityRuntime {
            update_checks: true,
        },
    ));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![RUNTIME_UPDATE_CHECK_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer)
            if answer.capabilities.0 == vec![RUNTIME_UPDATE_CHECK_CAPABILITY]
    ));
    write_json_frame(
        &mut client,
        &gent_protocol::RuntimeUpdateCheckFrame::Request(RuntimeUpdateCheckRequest {
            channel: RuntimeReleaseChannel::Stable,
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, gent_protocol::RuntimeUpdateCheckFrame>(&mut client)
            .await
            .unwrap(),
        gent_protocol::RuntimeUpdateCheckFrame::Report(RuntimeUpdateCheckReport {
            state: RuntimeUpdateCheckState::Current,
            ..
        })
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
