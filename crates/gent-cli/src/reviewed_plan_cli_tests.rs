use clap::Parser;
use gent_protocol::{
    Hello, Negotiated, REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatRunId, CapabilitySet, HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, PlanRevision,
    Receipt, ReceiptStatus, StartImplementationResult,
};
use tokio::net::UnixListener;

use super::{
    ContextArgument, EffortArgument, ModeArgument, ProviderArgument, ReviewedPlanCommand,
    StartArgs, execute,
};
use crate::{Args, CommandLine};

#[test]
fn start_parses_model_selection_and_clear_context() {
    let args = Args::try_parse_from([
        "gent",
        "plan",
        "start",
        "--conversation-id",
        "conversation-1",
        "--plan-id",
        "plan-1",
        "--plan-revision",
        "2",
        "--plan-content-digest-sha256",
        &"a".repeat(64),
        "--parent-run-id",
        "run-1",
        "--provider",
        "codex",
        "--model",
        "gpt-5.6",
        "--effort",
        "high",
        "--mode",
        "plan",
        "--context",
        "clear",
        "--policy-revision",
        "4",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Plan {
            action: ReviewedPlanCommand::Start(StartArgs {
                provider: ProviderArgument::Codex,
                effort: EffortArgument::High,
                mode: ModeArgument::Plan,
                context: ContextArgument::Clear,
                policy_revision: 4,
                ..
            })
        })
    ));
}

#[tokio::test]
async fn observer_mode_rejects_before_any_plan_frame_is_sent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|item| item == REVIEWED_PLAN_CAPABILITY)
        ));
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
    let error = execute(
        Some(directory.path().into()),
        true,
        ReviewedPlanCommand::Review(super::ReviewArgs {
            conversation_id: "conversation-1".into(),
            plan_id: "plan-1".into(),
            request_id: Some("review-1".into()),
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
}

#[tokio::test]
async fn start_binds_fresh_host_epoch_and_clear_context_to_the_reply() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![REVIEWED_PLAN_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::StatusRequest
        ));
        write_frame(
            &mut stream,
            &WireFrame::Status(HostStatus {
                host_epoch: HostEpoch(9),
                protocol_min: PROTOCOL_MIN,
                protocol_max: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![REVIEWED_PLAN_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let ReviewedPlanFrame::StartImplementation { request } =
            read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected plan start");
        };
        assert_eq!(request.host_epoch, HostEpoch(9));
        assert_eq!(request.context_policy, gent_types::ContextPolicy::Clear);
        assert_eq!(request.selection.model, "gpt-5.6");
        write_json_frame(
            &mut stream,
            &ReviewedPlanFrame::StartedImplementation {
                request_id: request.request_id.0.clone(),
                result: StartImplementationResult {
                    receipt: Receipt {
                        receipt_id: request.receipt_id,
                        idempotency_key: request.idempotency_key,
                        status: ReceiptStatus::Accepted,
                        host_epoch: HostEpoch(9),
                    },
                    conversation_id: request.conversation_id,
                    plan_id: request.plan_id,
                    plan_revision: request.plan_revision,
                    parent_run_id: request.parent_run_id,
                    implementation_run_id: AgentChatRunId("run-2".into()),
                    selection: request.selection,
                    context_policy: request.context_policy,
                    context_through_ordinal: 0,
                },
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ReviewedPlanCommand::Start(StartArgs {
            conversation_id: "conversation-1".into(),
            plan_id: "plan-1".into(),
            plan_revision: 2,
            plan_content_digest_sha256: "a".repeat(64),
            parent_run_id: "run-1".into(),
            provider: ProviderArgument::Codex,
            model: "gpt-5.6".into(),
            effort: EffortArgument::High,
            mode: ModeArgument::Plan,
            context: ContextArgument::Clear,
            policy_revision: 4,
            request_id: Some("start-1".into()),
            receipt_id: Some("receipt-1".into()),
            idempotency_key: Some("key-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        ReviewedPlanFrame::StartedImplementation { result, .. }
            if result.context_through_ordinal == 0 && result.implementation_run_id.0 == "run-2"
    ));
}

#[test]
fn plan_rejection_remains_exact_revision_bound() {
    let frame = super::frame(ReviewedPlanCommand::Reject(super::RejectArgs {
        plan_id: "plan-1".into(),
        plan_revision: 2,
        plan_content_digest_sha256: "a".repeat(64),
        request_id: Some("reject-1".into()),
    }));
    assert!(matches!(
        frame,
        ReviewedPlanFrame::Reject {
            plan_revision: PlanRevision(2),
            ..
        }
    ));
}
