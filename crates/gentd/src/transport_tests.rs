use gent_protocol::{
    CONVERSATION_STATUS_CAPABILITY, ConversationStatusFrame, DecisionEvidence, DependencyAction,
    DependencyActionRequest, DependencyActionState, DependencyPlan, DependencyPlanRequest,
    DependencyProvider, Hello, PublicRunOutcome, PublicRunResponse, PublicRunStartRequest,
    WireFrame, read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, DoctorReport,
    HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN,
};
use tokio::io::duplex;

use crate::api::RuntimeApi;
use crate::transport::serve_connection;

#[derive(Clone, Debug)]
struct FakeRuntime;

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
        DependencyPlan {
            provider: request.provider,
            action: request.action,
            instruction: "review vendor installer".into(),
            consent_required: true,
        }
    }
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> gent_protocol::DependencyActionResult {
        gent_protocol::DependencyActionResult {
            plan: self.dependency_plan(DependencyPlanRequest {
                provider: request.provider,
                action: request.action,
            }),
            state: if request.consent_granted {
                DependencyActionState::InstallerNotConfigured
            } else {
                DependencyActionState::ConsentRequired
            },
        }
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
    fn apply_decision_evidence(
        &self,
        decision_id: String,
        evidence: DecisionEvidence,
    ) -> Result<DecisionSettlement, String> {
        let phase = match evidence {
            DecisionEvidence::ProviderAcknowledged => DecisionSettlementPhase::Acknowledged,
            DecisionEvidence::ProviderSettled => DecisionSettlementPhase::Settled,
            DecisionEvidence::AcknowledgementUnprovable => DecisionSettlementPhase::Unprovable,
            DecisionEvidence::RecoveryRequired => DecisionSettlementPhase::RecoveryRequired,
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
}

fn hello() -> WireFrame {
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
async fn decision_and_provider_lifecycle_are_routed_after_handshake() {
    let (mut client, server) = duplex(1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(&mut client, &hello()).await.unwrap();
    let _ = read_frame(&mut client).await.unwrap();
    write_frame(
        &mut client,
        &WireFrame::DecisionEvidence {
            decision_id: "decision-1".into(),
            evidence: DecisionEvidence::AcknowledgementUnprovable,
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut client).await.unwrap(), WireFrame::DecisionSettlement(decision) if decision.phase == DecisionSettlementPhase::Unprovable)
    );
    let run = PublicRunStartRequest {
        run_id: "run".into(),
        coordinator_id: "host".into(),
        host_epoch: HostEpoch(1),
        provider: DependencyProvider::Claude,
        executable: "/tmp/claude".into(),
        version: "1".into(),
        compatibility_entry: "fixture".into(),
    };
    write_frame(&mut client, &WireFrame::PublicRunStart(run))
        .await
        .unwrap();
    assert!(
        matches!(read_frame(&mut client).await.unwrap(), WireFrame::PublicRunResponse(response) if response.outcome == PublicRunOutcome::Denied)
    );
    drop(client);
    assert!(task.await.unwrap().is_err());
}
