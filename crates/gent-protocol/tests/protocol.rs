use gent_protocol::{
    DependencyAction, DependencyPlanRequest, DependencyProvider, Hello, PublicRunOutcome,
    PublicRunResponse, WireFrame, negotiate, read_frame, write_frame,
};
use gent_types::CapabilitySet;
use tokio::io::duplex;

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
