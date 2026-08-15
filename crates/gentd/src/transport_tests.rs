use gent_protocol::{
    CONVERSATION_STATUS_CAPABILITY, ConversationStatusFrame, DecisionRecoveryEvidence,
    DependencyAction, DependencyActionRequest, DependencyActionState, DependencyPlan,
    DependencyPlanRequest, DependencyProvider, Hello, PublicRunOutcome, PublicRunResponse,
    PublicRunStartRequest, WireFrame, read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, DoctorReport,
    HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt, ReceiptId, ReceiptStatus,
};
use tokio::io::duplex;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

#[derive(Clone, Debug)]
pub(crate) struct FakeRuntime;

impl RuntimeApi for FakeRuntime {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        Ok(CapabilitySet(vec![
            CONVERSATION_STATUS_CAPABILITY.into(),
            "event-resync".into(),
            "events".into(),
        ]))
    }

    fn status(&self) -> Result<HostStatus, String> {
        Ok(HostStatus {
            host_epoch: HostEpoch(1),
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: self.capabilities()?,
        })
    }
    fn submit(&self, _: gent_types::Command) -> Result<gent_types::Receipt, String> {
        Err("not used".into())
    }
    fn resume_events(&self, _: u64) -> Result<gent_types::EventResume, String> {
        Ok(gent_types::EventResume::Delta { events: Vec::new() })
    }
    fn doctor(&self) -> DoctorReport {
        DoctorReport::empty()
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        DependencyPlan::reviewed(
            request.provider,
            request.action,
            "review vendor installer",
            true,
        )
    }
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<gent_protocol::DependencyActionResult, String> {
        Ok(gent_protocol::DependencyActionResult {
            plan: self.dependency_plan(DependencyPlanRequest {
                provider: request.provider,
                action: request.action,
            }),
            state: if request.consent_granted {
                DependencyActionState::Completed
            } else {
                DependencyActionState::ConsentRequired
            },
            receipt: Receipt {
                receipt_id: ReceiptId("fake".into()),
                idempotency_key: request.idempotency_key,
                status: ReceiptStatus::Rejected,
                host_epoch: request.host_epoch,
            },
            detail: None,
        })
    }
    fn submit_decision(
        &self,
        command: DecisionCommand,
    ) -> Result<gent_protocol::DecisionSubmission, String> {
        Ok(gent_protocol::DecisionSubmission::Accepted(
            DecisionSettlement {
                decision_id: command.decision_id,
                idempotency_key: command.idempotency_key,
                phase: DecisionSettlementPhase::Pending,
            },
        ))
    }
    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String> {
        let phase = match evidence {
            DecisionRecoveryEvidence::AcknowledgementUnprovable => {
                DecisionSettlementPhase::Unprovable
            }
            DecisionRecoveryEvidence::RecoveryRequired => DecisionSettlementPhase::RecoveryRequired,
        };
        Ok(DecisionSettlement {
            decision_id,
            idempotency_key: "fake".into(),
            phase,
        })
    }
    fn start_public_run(
        &self,
        request: PublicRunStartRequest,
    ) -> Result<PublicRunResponse, String> {
        Ok(PublicRunResponse {
            run_id: request.run_id,
            outcome: PublicRunOutcome::Denied,
        })
    }
    fn resume_public_run(
        &self,
        request: gent_protocol::PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, String> {
        Ok(PublicRunResponse {
            run_id: request.run_id,
            outcome: PublicRunOutcome::Denied,
        })
    }
    fn interrupt_public_run(
        &self,
        request: gent_protocol::PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String> {
        Ok(PublicRunResponse {
            run_id: request.run_id,
            outcome: PublicRunOutcome::Denied,
        })
    }
    fn conversation_status(
        &self,
        conversation_id: &str,
    ) -> Result<gent_types::ConversationStatus, String> {
        Ok(gent_types::ConversationStatus {
            conversation_id: conversation_id.into(),
            runs: Vec::new(),
        })
    }
    fn conversation_timeline(
        &self,
        conversation_id: &str,
    ) -> Result<gent_types::ConversationTimeline, String> {
        Ok(gent_types::ConversationTimeline {
            conversation_id: conversation_id.into(),
            runs: Vec::new(),
            artifacts: Vec::new(),
        })
    }
}

pub(crate) fn hello() -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec!["event-resync".into(), "events".into()]),
    })
}

fn conversation_hello() -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec![CONVERSATION_STATUS_CAPABILITY.into()]),
    })
}

#[tokio::test]
async fn handshake_is_mandatory_before_requests() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &WireFrame::StatusRequest)
        .await
        .unwrap();
    assert!(
        matches!(read_frame(&mut client).await.unwrap(), WireFrame::Error { code, .. } if code == "handshakeRequired")
    );
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn conversation_status_uses_the_negotiated_extension_without_a_receipt() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &conversation_hello())
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer)
            if answer.capabilities.0 == vec![CONVERSATION_STATUS_CAPABILITY]
    ));
    write_json_frame(
        &mut client,
        &ConversationStatusFrame::Request {
            conversation_id: "conversation-1".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, ConversationStatusFrame>(&mut client)
            .await
            .unwrap(),
        ConversationStatusFrame::Status(status) if status.conversation_id == "conversation-1"
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn conversation_status_is_rejected_without_its_negotiated_capability() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_json_frame(
        &mut client,
        &ConversationStatusFrame::Request {
            conversation_id: "conversation-1".into(),
        },
    )
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
async fn typed_dependency_requests_need_consent_and_never_start_an_installer() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(answer) if answer.capabilities == CapabilitySet(vec!["event-resync".into(), "events".into()])
    ));
    write_frame(
        &mut client,
        &WireFrame::DependencyActionRequest(DependencyActionRequest {
            provider: DependencyProvider::Claude,
            action: DependencyAction::Install,
            consent_granted: false,
            receipt_id: ReceiptId("fake".into()),
            idempotency_key: "fake".into(),
            host_epoch: HostEpoch(1),
            reviewed_plan_digest: "fake".into(),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut client).await.unwrap(), WireFrame::DependencyActionResult(result) if result.state == DependencyActionState::ConsentRequired)
    );
    drop(client);
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn onboarding_is_read_only_and_returns_the_closed_provider_model() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_frame(&mut client, &WireFrame::OnboardingRequest)
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Onboarding(state)
            if state.branches.len() == 3
                && state.branches[0].provider == gent_types::OnboardingProvider::Gent
    ));
    drop(client);
    assert!(task.await.unwrap().is_err());
}
