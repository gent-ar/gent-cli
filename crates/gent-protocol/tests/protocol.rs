use gent_protocol::{
    CONVERSATION_INDEX_CAPABILITY, CONVERSATION_STATUS_CAPABILITY, ConversationIndexFrame,
    ConversationStatusFrame, DependencyAction, DependencyPlan, DependencyPlanRequest,
    DependencyProvider, EXTERNAL_PROVIDER_BRIDGE_CAPABILITY, ExternalProviderBridgeFrame,
    ExternalProviderBridgeHello, Hello, MAX_FRAME_BYTES, PublicRunOutcome, PublicRunResponse,
    RUNTIME_UPDATE_CHECK_CAPABILITY, RuntimeUpdateCheckFrame, WireFrame, negotiate, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    CapabilitySet, RuntimeReleaseChannel, RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest,
    RuntimeUpdateCheckState, RuntimeVersion,
};
use tokio::io::{AsyncWriteExt, duplex};

#[test]
fn negotiation_intersects_capabilities() {
    let hello = Hello {
        protocol_min: 1,
        protocol_max: 2,
        capabilities: CapabilitySet(vec!["events".into(), "future".into()]),
    };
    let answer = negotiate(
        &hello,
        1,
        1,
        &CapabilitySet(vec!["events".into(), "receipts".into()]),
    )
    .unwrap();
    assert_eq!(answer.protocol, 1);
    assert_eq!(answer.capabilities, CapabilitySet(vec!["events".into()]));
}

#[tokio::test]
async fn frames_round_trip_and_ignore_additive_fields() {
    let (mut writer, mut reader) = duplex(1024);
    let frame = WireFrame::Hello(Hello {
        protocol_min: 1,
        protocol_max: 1,
        capabilities: CapabilitySet::default(),
    });
    write_frame(&mut writer, &frame).await.unwrap();
    assert_eq!(read_frame(&mut reader).await.unwrap(), frame);
    let body = br#"{"type":"hello","body":{"protocolMin":1,"protocolMax":1,"capabilities":[],"futureField":true}}"#;
    assert_eq!(serde_json::from_slice::<WireFrame>(body).unwrap(), frame);
}

#[test]
fn onboarding_frames_are_additive_and_read_only() {
    let frame = WireFrame::OnboardingRequest;
    let encoded = serde_json::to_string(&frame).unwrap();
    assert_eq!(serde_json::from_str::<WireFrame>(&encoded).unwrap(), frame);
    assert!(encoded.contains("onboardingRequest"));
}

#[tokio::test]
async fn additive_conversation_frames_share_the_bounded_json_framing() {
    let (mut writer, mut reader) = duplex(1024);
    let frame = ConversationStatusFrame::Request {
        conversation_id: "conversation-1".into(),
    };
    write_json_frame(&mut writer, &frame).await.unwrap();
    assert_eq!(
        read_json_frame::<_, ConversationStatusFrame>(&mut reader)
            .await
            .unwrap(),
        frame
    );

    let (mut writer, mut reader) = duplex(16);
    writer
        .write_u32(u32::try_from(MAX_FRAME_BYTES + 1).unwrap())
        .await
        .unwrap();
    assert!(
        read_json_frame::<_, ConversationStatusFrame>(&mut reader)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn conversation_index_is_an_explicit_content_free_extension() {
    let (mut writer, mut reader) = duplex(1024);
    let frame = ConversationIndexFrame::Index(vec![gent_types::ConversationListItem {
        conversation_id: "conversation-1".into(),
        run_count: 1,
    }]);
    write_json_frame(&mut writer, &frame).await.unwrap();
    assert_eq!(
        read_json_frame::<_, ConversationIndexFrame>(&mut reader)
            .await
            .unwrap(),
        frame
    );
    assert_eq!(CONVERSATION_INDEX_CAPABILITY, "conversation-index-v1");
}

#[tokio::test]
async fn private_bridge_frames_share_bounded_framing_without_public_wire_variants() {
    let (mut writer, mut reader) = duplex(1024);
    let frame = ExternalProviderBridgeFrame::Hello(ExternalProviderBridgeHello {
        protocol_min: 1,
        protocol_max: 1,
        capabilities: CapabilitySet(vec![EXTERNAL_PROVIDER_BRIDGE_CAPABILITY.into()]),
    });

    write_json_frame(&mut writer, &frame).await.unwrap();
    assert_eq!(
        read_json_frame::<_, ExternalProviderBridgeFrame>(&mut reader)
            .await
            .unwrap(),
        frame
    );
    assert!(!serde_json::to_string(&frame).unwrap().contains("claurst"));
}

#[test]
fn conversation_status_capability_is_an_explicit_wire_contract() {
    assert_eq!(CONVERSATION_STATUS_CAPABILITY, "conversation-status-v1");
}

#[test]
fn public_provider_frames_exclude_private_bridges() {
    assert!(matches!("claude".parse(), Ok(DependencyProvider::Claude)));
    assert!(matches!("update".parse(), Ok(DependencyAction::Update)));
    assert!("claurst".parse::<DependencyProvider>().is_err());
    let dependency = WireFrame::DependencyPlanRequest(DependencyPlanRequest {
        provider: DependencyProvider::Codex,
        action: DependencyAction::Install,
    });
    assert!(
        serde_json::to_string(&dependency)
            .unwrap()
            .contains("codex")
    );
    let response = WireFrame::PublicRunResponse(PublicRunResponse {
        run_id: "run".into(),
        outcome: PublicRunOutcome::Denied,
    });
    assert_eq!(
        serde_json::from_str::<WireFrame>(&serde_json::to_string(&response).unwrap()).unwrap(),
        response
    );
}

#[test]
fn reviewed_dependency_plan_digest_binds_every_visible_plan_field() {
    let plan = DependencyPlan::reviewed(
        DependencyProvider::Claude,
        DependencyAction::Install,
        "review vendor installer",
        true,
    );
    let changed = DependencyPlan::reviewed(
        DependencyProvider::Claude,
        DependencyAction::Update,
        "review vendor installer",
        true,
    );
    assert_eq!(plan.reviewed_plan_digest.len(), 64);
    assert_ne!(plan.reviewed_plan_digest, changed.reviewed_plan_digest);
}

#[tokio::test]
async fn update_check_is_a_capability_guarded_read_only_extension() {
    let (mut writer, mut reader) = duplex(1024);
    let frame = RuntimeUpdateCheckFrame::Report(RuntimeUpdateCheckReport {
        current_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        state: RuntimeUpdateCheckState::Current,
        candidate: None,
        failure: None,
    });
    write_json_frame(&mut writer, &frame).await.unwrap();
    assert_eq!(
        read_json_frame::<_, RuntimeUpdateCheckFrame>(&mut reader)
            .await
            .unwrap(),
        frame
    );
    assert_eq!(RUNTIME_UPDATE_CHECK_CAPABILITY, "runtime-update-check-v1");
    let request = RuntimeUpdateCheckFrame::Request(RuntimeUpdateCheckRequest {
        channel: RuntimeReleaseChannel::Beta,
    });
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(encoded.contains("request"));
    assert!(!encoded.contains("activate"));
    assert!(!encoded.contains("stage"));
}
