use std::sync::{Arc, Mutex};

use gent_ports::{IngressMode, Ledger};
use gent_protocol::{
    Hello, WireFrame, read_frame, read_json_frame,
    runtime_owner::{RUNTIME_OWNER_CONTROL_CAPABILITY, RuntimeOwnerFrame},
    write_frame, write_json_frame,
};
use gent_runtime::{
    AgentChatReadService,
    catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile},
};
use gent_types::{CapabilitySet, HostEpoch, PROTOCOL_MAX, PROTOCOL_MIN};

use super::*;
use crate::{
    ordinary_lifecycle_cadence::pair, ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter,
};

fn runtime() -> (tempfile::TempDir, RuntimeFacade, OrdinaryLifecycleControl) {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = crate::runtime_facade::build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::RuntimeOwnerControl]),
        crate::CompatibilityAssessment::load(None, &[], 1),
    )
    .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(runtime.local_model_events.clone()),
            vec![],
        )
        .unwrap(),
    ));
    let (control, ingress, _) = pair(router);
    control.open_after_recovery();
    runtime.ordinary_prompt_ingress = Some(ingress);
    (directory, runtime, control)
}

fn request(port: &RuntimeOwnerControlPort) -> RuntimeOwnerFrame {
    let RuntimeOwnerFrame::RuntimeOwnerState {
        host_epoch,
        process_id,
        executable_digest_sha256,
        ..
    } = port
        .exchange(RuntimeOwnerFrame::RuntimeOwnerStatus {
            request_id: "status".into(),
        })
        .unwrap()
    else {
        panic!("expected exact owner status")
    };
    RuntimeOwnerFrame::QuiesceRuntimeOwner {
        request_id: "quiesce".into(),
        expected_host_epoch: host_epoch,
        expected_process_id: process_id,
        expected_executable_digest_sha256: executable_digest_sha256,
    }
}

#[test]
fn exact_owner_quiesce_closes_durable_and_transient_admission() {
    let (_directory, runtime, control) = runtime();
    let port = runtime.owner_control_port().unwrap();
    let frame = request(&port);
    assert!(matches!(
        port.exchange(frame.clone()).unwrap(),
        RuntimeOwnerFrame::RuntimeOwnerQuiesced { .. }
    ));
    assert_eq!(
        runtime.local_model_events.host_ingress().unwrap().mode,
        IngressMode::Closed
    );
    assert!(control.acquire_prompt().is_err());
    assert!(port.exchange(frame).is_ok());
}

#[test]
fn typed_pre_ledger_rejection_proves_only_this_request_not_applied() {
    use gent_protocol::runtime_owner::RuntimeOwnerQuiesceEffect as Effect;
    let (_directory, runtime, control) = runtime();
    let port = runtime.owner_control_port().unwrap();
    let frame = request(&port);
    let _permit = control.acquire_operation().unwrap();
    let failure = port.exchange_typed(frame).unwrap_err();
    assert_eq!(failure.effect, Effect::NotApplied);
    assert_eq!(
        runtime.local_model_events.host_ingress().unwrap().mode,
        IngressMode::Open
    );
    assert!(!control.is_quiesced());
}

#[test]
fn typed_ledger_failure_preserves_ambiguity_instead_of_claiming_no_effect() {
    use gent_protocol::runtime_owner::RuntimeOwnerQuiesceEffect as Effect;
    let (_directory, runtime, control) = runtime();
    let port = runtime.owner_control_port().unwrap();
    let frame = request(&port);
    runtime
        .local_model_events
        .close_ingress(HostEpoch(1))
        .unwrap();
    runtime
        .local_model_events
        .fence_and_open(HostEpoch(1))
        .unwrap();
    let failure = port.exchange_typed(frame).unwrap_err();
    assert_eq!(failure.effect, Effect::MayHaveChanged);
    assert!(!control.is_quiesced());
}

#[test]
fn stale_owner_tuple_never_closes_or_stops_current_instance() {
    for mismatch in 0..3 {
        let (_directory, runtime, control) = runtime();
        let port = runtime.owner_control_port().unwrap();
        let mut frame = request(&port);
        if let RuntimeOwnerFrame::QuiesceRuntimeOwner {
            expected_host_epoch,
            expected_process_id,
            expected_executable_digest_sha256,
            ..
        } = &mut frame
        {
            match mismatch {
                0 => *expected_host_epoch = HostEpoch(expected_host_epoch.0 + 1),
                1 => *expected_process_id = expected_process_id.wrapping_add(1),
                _ => *expected_executable_digest_sha256 = "0".repeat(64),
            }
        }
        assert!(port.exchange(frame).is_err());
        assert!(!control.is_quiesced());
        assert_eq!(
            runtime.local_model_events.host_ingress().unwrap().mode,
            IngressMode::Open
        );
    }
}

#[test]
fn active_native_operation_rejects_repair_but_preserves_later_retry() {
    let (_directory, runtime, control) = runtime();
    let port = runtime.owner_control_port().unwrap();
    let operation = port.acquire_operation().unwrap();
    let frame = request(&port);
    assert!(port.exchange(frame.clone()).is_err());
    assert!(!control.is_quiesced());
    drop(operation);
    assert!(port.exchange(frame).is_ok());
}

#[tokio::test]
async fn negotiated_local_transport_flushes_exact_quiesce_ack() {
    let (_directory, runtime, control) = runtime();
    let frame = request(&runtime.owner_control_port().unwrap());
    let (mut client, server) = tokio::io::duplex(4096);
    let task = tokio::spawn(crate::transport::serve_connection(server, runtime));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![RUNTIME_OWNER_CONTROL_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(_)
    ));
    write_json_frame(&mut client, &frame).await.unwrap();
    let response: serde_json::Value = read_json_frame(&mut client).await.unwrap();
    assert!(matches!(
        serde_json::from_value::<RuntimeOwnerFrame>(response).unwrap(),
        RuntimeOwnerFrame::RuntimeOwnerQuiesced { .. }
    ));
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        control.quiesce_acknowledged(),
    )
    .await
    .unwrap();
    drop(client);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn typed_no_effect_rejection_requires_its_negotiated_capability() {
    use gent_protocol::runtime_owner::{
        RUNTIME_OWNER_QUIESCE_REJECTION_CAPABILITY, RuntimeOwnerQuiesceEffect as Effect,
    };
    for typed in [false, true] {
        let (_directory, runtime, control) = runtime();
        let frame = request(&runtime.owner_control_port().unwrap());
        let _permit = control.acquire_operation().unwrap();
        let (mut client, server) = tokio::io::duplex(4096);
        let task = tokio::spawn(crate::transport::serve_connection(server, runtime));
        let mut capabilities = vec![RUNTIME_OWNER_CONTROL_CAPABILITY.into()];
        if typed {
            capabilities.push(RUNTIME_OWNER_QUIESCE_REJECTION_CAPABILITY.into());
        }
        write_frame(
            &mut client,
            &WireFrame::Hello(Hello {
                protocol_min: PROTOCOL_MIN,
                protocol_max: PROTOCOL_MAX,
                capabilities: CapabilitySet(capabilities),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Negotiated(_)
        ));
        write_json_frame(&mut client, &frame).await.unwrap();
        let response: serde_json::Value = read_json_frame(&mut client).await.unwrap();
        if typed {
            let RuntimeOwnerFrame::RuntimeOwnerQuiesceRejected {
                request_id,
                failure,
            } = serde_json::from_value(response).unwrap()
            else {
                panic!("missing typed rejection")
            };
            assert_eq!(request_id, "quiesce");
            assert_eq!(failure.effect, Effect::NotApplied);
        } else {
            let WireFrame::Error { code, .. } = serde_json::from_value(response).unwrap() else {
                panic!("legacy generic rejection changed")
            };
            assert_eq!(code, "runtimeOwnerQuiesceRejected");
        }
        assert!(!control.is_quiesced());
        drop(client);
        task.await.unwrap().unwrap();
    }
}
