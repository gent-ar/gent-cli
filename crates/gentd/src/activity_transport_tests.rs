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
    CapabilitySet, Command, ConversationActivityFact, ConversationActivityPage,
    ConversationActivityScope, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventPage, HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN,
    Receipt, RuntimeMaintenanceReport, RuntimeMaintenanceRequest, RuntimeReleaseChannel,
    RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState,
    RuntimeUpdateHandoff, RuntimeUpdateRecord, RuntimeUpdateStatus, RuntimeVersion,
};
use tokio::io::duplex;

use crate::{
    api::RuntimeApi,
    transport::{observed_capabilities, serve_connection},
};

#[derive(Clone, Debug)]
struct ActivityRuntime {
    update_checks: bool,
    maintenance: bool,
}

impl RuntimeApi for ActivityRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        let mut capabilities = vec![CONVERSATION_ACTIVITY_CAPABILITY.into()];
        if self.update_checks {
            capabilities.push(RUNTIME_UPDATE_CHECK_CAPABILITY.into());
        }
        if self.maintenance {
            capabilities.push(gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY.into());
        }
        Ok(CapabilitySet(capabilities))
    }

    fn status(&self) -> Result<HostStatus, String> {
        Err("not used".into())
    }
    fn submit(&self, _: Command) -> Result<Receipt, String> {
        Err("not used".into())
    }
    fn read_event_page(&self, _: u64, _: usize) -> Result<EventPage, String> {
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
    fn runtime_maintenance(
        &self,
        request: RuntimeMaintenanceRequest,
    ) -> Result<RuntimeMaintenanceReport, String> {
        Ok(RuntimeMaintenanceReport {
            host_epoch: HostEpoch(4),
            ingress_closed: false,
            record: RuntimeUpdateRecord {
                attempt_id: request.attempt_id,
                revision: 2,
                artifact_digest_sha256: "a".repeat(64),
                status: RuntimeUpdateStatus::default(),
                handoff: RuntimeUpdateHandoff::default(),
            },
        })
    }
    fn conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        _: u64,
    ) -> Result<ConversationActivityRead, String> {
        Ok(ConversationActivityRead::Page(ConversationActivityPage {
            facts: vec![ConversationActivityFact::TurnStarted {
                scope: ConversationActivityScope {
                    conversation_id: conversation_id.into(),
                    run_id: run_id.into(),
                    turn_id: "turn-1".into(),
                    host_epoch: HostEpoch(4),
                    cursor: 8,
                },
            }],
            next_after_cursor: None,
        }))
    }
}

#[test]
fn observer_capabilities_do_not_advertise_authority_or_update_work() {
    let capabilities = observed_capabilities(false, false, false, false);
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
async fn activity_facts_require_a_negotiated_activity_capability() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(
        server,
        ActivityRuntime {
            update_checks: false,
            maintenance: false,
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
        ConversationActivityFrame::Facts(page)
            if page.facts[0].scope().cursor == 8 && page.next_after_cursor.is_none()
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
            maintenance: false,
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

#[tokio::test]
async fn maintenance_report_requires_the_negotiated_authority_capability() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(
        server,
        ActivityRuntime {
            update_checks: false,
            maintenance: true,
        },
    ));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut client).await.unwrap(), WireFrame::Negotiated(answer)
        if answer.capabilities.0 == vec![gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY])
    );
    write_json_frame(
        &mut client,
        &gent_protocol::RuntimeMaintenanceFrame::Request(RuntimeMaintenanceRequest {
            attempt_id: "attempt-1".into(),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, gent_protocol::RuntimeMaintenanceFrame>(&mut client).await.unwrap(),
        gent_protocol::RuntimeMaintenanceFrame::Report(report)
            if report.record.attempt_id == "attempt-1" && report.record.revision == 2
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
