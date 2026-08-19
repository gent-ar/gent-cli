use gent_protocol::{
    Hello, Negotiated, PROMPT_PROVIDER_PROVISION_CAPABILITY, PROVIDER_READINESS_CAPABILITY,
    PromptProviderProvisionFrame, PromptProviderProvisionState, ProviderReadinessFrame, WireFrame,
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatProvider, CapabilitySet, HostEpoch, HostStatus, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::{ProvisionArgs, provision, provision_request, readiness};

#[tokio::test]
async fn readiness_refuses_observer_mode_before_sending_a_request() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(_)
        ));
        write_frame(&mut stream, &negotiated(vec![])).await.unwrap();
    });
    let error = readiness(
        Some(directory.path().into()),
        true,
        "conversation-1",
        "run-1",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
    server.await.unwrap();
}

#[tokio::test]
async fn readiness_sends_only_the_accepted_run_identity() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|value| value == PROVIDER_READINESS_CAPABILITY)
        ));
        write_frame(
            &mut stream,
            &negotiated(vec![PROVIDER_READINESS_CAPABILITY.into()]),
        )
        .await
        .unwrap();
        let request: ProviderReadinessFrame = read_json_frame(&mut stream).await.unwrap();
        let ProviderReadinessFrame::Assess {
            conversation_id,
            run_id,
        } = request
        else {
            panic!("terminal must only assess readiness");
        };
        assert_eq!(conversation_id.0, "conversation-1");
        assert_eq!(run_id.0, "run-1");
        write_json_frame(
            &mut stream,
            &ProviderReadinessFrame::Ready {
                conversation_id,
                run_id,
                provider: AgentChatProvider::Codex,
            },
        )
        .await
        .unwrap();
    });
    assert!(matches!(
        readiness(
            Some(directory.path().into()),
            true,
            "conversation-1",
            "run-1"
        )
        .await
        .unwrap(),
        ProviderReadinessFrame::Ready {
            provider: AgentChatProvider::Codex,
            ..
        }
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn provision_uses_daemon_epoch_and_never_constructs_provider_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(_)
        ));
        write_frame(
            &mut stream,
            &negotiated(vec![PROMPT_PROVIDER_PROVISION_CAPABILITY.into()]),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::StatusRequest
        ));
        write_frame(&mut stream, &WireFrame::Status(status()))
            .await
            .unwrap();
        let request: PromptProviderProvisionFrame = read_json_frame(&mut stream).await.unwrap();
        let encoded = serde_json::to_value(&request).unwrap();
        assert!(encoded.pointer("/body/provider").is_none());
        assert!(encoded.pointer("/body/package").is_none());
        let PromptProviderProvisionFrame::Confirm {
            receipt_id,
            idempotency_key,
            host_epoch,
            prompt_receipt_id,
            conversation_id,
            run_id,
            consent_granted,
            reviewed_plan_digest,
        } = request
        else {
            panic!("terminal must only confirm a daemon review");
        };
        assert_eq!(host_epoch, HostEpoch(7));
        assert!(consent_granted);
        assert_eq!(reviewed_plan_digest, "a".repeat(64));
        write_json_frame(
            &mut stream,
            &PromptProviderProvisionFrame::Result {
                receipt: Receipt {
                    receipt_id,
                    idempotency_key,
                    status: ReceiptStatus::Settled,
                    host_epoch,
                },
                prompt_receipt_id,
                conversation_id,
                run_id,
                state: PromptProviderProvisionState::Completed,
            },
        )
        .await
        .unwrap();
    });
    assert!(matches!(
        provision(Some(directory.path().into()), true, provision_args(),)
            .await
            .unwrap(),
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Completed,
            ..
        }
    ));
    server.await.unwrap();
}

#[test]
fn provision_retries_reuse_the_exact_receipt_for_one_idempotency_key() {
    let first = provision_request(provision_args(), HostEpoch(7)).unwrap();
    let second = provision_request(provision_args(), HostEpoch(7)).unwrap();
    let (
        PromptProviderProvisionFrame::Confirm {
            receipt_id: first_receipt,
            idempotency_key: first_key,
            ..
        },
        PromptProviderProvisionFrame::Confirm {
            receipt_id: second_receipt,
            idempotency_key: second_key,
            ..
        },
    ) = (first, second)
    else {
        panic!("terminal must construct confirmations");
    };
    assert_eq!(first_key, "provision-key-1");
    assert_eq!(first_key, second_key);
    assert_eq!(first_receipt, second_receipt);
    assert!(first_receipt.0.starts_with("prompt-provider-provision-"));
}

fn provision_args() -> ProvisionArgs {
    ProvisionArgs {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        prompt_receipt_id: "prompt-receipt-1".into(),
        reviewed_plan_digest: "a".repeat(64),
        consent: true,
        idempotency_key: Some("provision-key-1".into()),
    }
}

fn negotiated(capabilities: Vec<String>) -> WireFrame {
    WireFrame::Negotiated(Negotiated {
        protocol: PROTOCOL_MAX,
        capabilities: CapabilitySet(capabilities),
    })
}

fn status() -> HostStatus {
    HostStatus {
        host_epoch: HostEpoch(7),
        protocol_min: PROTOCOL_MAX,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet::default(),
    }
}
