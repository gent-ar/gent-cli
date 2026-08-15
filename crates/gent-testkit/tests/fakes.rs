use gent_ports::ExternalProviderBridge;
use gent_testkit::{FakeExternalProviderBridge, FakeProcess, FakeProcessSignal};
use gent_types::{CapabilitySet, Command, HostEpoch, ProviderEvent, ReceiptId};
use serde_json::json;

fn command() -> Command {
    Command {
        receipt_id: ReceiptId("receipt".into()),
        idempotency_key: "key".into(),
        host_epoch: HostEpoch(4),
        kind: "submit".into(),
        payload: json!({}),
    }
}

#[tokio::test]
async fn bridge_consumes_its_script_in_order_and_records_sessions() {
    let bridge = FakeExternalProviderBridge::default();
    bridge.push_event(ProviderEvent::Output {
        text: "first".into(),
    });
    bridge.fail_next_event("offline");
    bridge.fail_next_submit("unavailable");

    assert!(bridge.submit("session-a", command()).await.is_err());
    assert_eq!(bridge.submissions()[0].opaque_session, "session-a");
    assert!(matches!(
        bridge.next_event("session-a").await,
        Ok(Some(ProviderEvent::Output { .. }))
    ));
    assert!(bridge.next_event("session-a").await.is_err());
    assert!(bridge.next_event("session-a").await.unwrap().is_none());
    bridge.set_capabilities(CapabilitySet(vec!["private-bridge".into()]));
    assert_eq!(
        bridge.register_capabilities().await.unwrap().0,
        ["private-bridge"]
    );
    assert_eq!(
        bridge.start_run("run-a").await.unwrap().opaque_session,
        "bridge-run-a"
    );
    assert_eq!(bridge.resume_run("run-a").await.unwrap().run_id, "run-a");
    bridge.interrupt("session-a").await.unwrap();
    assert!(bridge.terminal_state("session-a").await.unwrap().is_none());
}

#[test]
fn process_preserves_fifo_io_and_records_lifecycle_facts() {
    let process = FakeProcess::default();
    process.write_stdin(b"input".to_vec());
    process.push_stdout(b"one".to_vec());
    process.push_stdout(b"two".to_vec());
    process.push_stderr(b"warning".to_vec());
    process.signal(FakeProcessSignal::Interrupt);
    process.exit(130);

    assert_eq!(process.stdin(), vec![b"input".to_vec()]);
    assert_eq!(process.read_stdout(), Some(b"one".to_vec()));
    assert_eq!(process.read_stdout(), Some(b"two".to_vec()));
    assert_eq!(process.read_stderr(), Some(b"warning".to_vec()));
    assert_eq!(process.signals(), vec![FakeProcessSignal::Interrupt]);
    assert_eq!(process.exit_code(), Some(130));
}
