//! Phase 2: whole-profile proof for prompt-provider-provision, entirely outside
//! `daemon_bootstrap.rs`. Drives the real composed `RuntimeFacade` over the real wire codec.

use gent_ports::ProvisionedProviderLockLedger;
use gent_protocol::{
    Hello, PROMPT_PROVIDER_PROVISION_CAPABILITY, PromptProviderProvisionFrame,
    PromptProviderProvisionState, WireFrame, read_frame, read_json_frame, write_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, HostEpoch, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId,
};
use tokio::io::{AsyncRead, AsyncWrite, duplex};

use crate::prompt_provider_provision_profile_support::{
    AmbiguousVerifier, FakeVerifier, Profile, compose, plan_digest,
};
use crate::transport::serve_connection;

fn hello() -> WireFrame {
    WireFrame::Hello(Hello {
        protocol_min: PROTOCOL_MIN,
        protocol_max: PROTOCOL_MAX,
        capabilities: CapabilitySet(vec![
            "events".into(),
            PROMPT_PROVIDER_PROVISION_CAPABILITY.into(),
        ]),
    })
}

async fn attach<S>(stream: &mut S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_frame(stream, &hello()).await.unwrap();
    assert!(matches!(
        read_frame(stream).await.unwrap(),
        WireFrame::Negotiated(answer) if answer.protocol == PROTOCOL_MAX
    ));
}

fn confirm(
    profile: &Profile,
    reviewed_plan_digest: String,
    consent_granted: bool,
) -> PromptProviderProvisionFrame {
    PromptProviderProvisionFrame::Confirm {
        receipt_id: ReceiptId("provision-receipt".into()),
        idempotency_key: "provision-key".into(),
        host_epoch: HostEpoch(1),
        prompt_receipt_id: profile.saved.receipt.receipt_id.clone(),
        conversation_id: AgentChatConversationId(profile.saved.message.conversation_id.clone()),
        run_id: profile.saved.run_id.clone(),
        consent_granted,
        reviewed_plan_digest,
    }
}

async fn confirm_over<S>(stream: &mut S, profile: &Profile, digest: String, consent: bool)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_json_frame(stream, &confirm(profile, digest, consent))
        .await
        .unwrap();
}

#[tokio::test]
async fn confirm_persists_before_broadcast_and_reconnect_replays_from_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let profile = compose(directory.path(), FakeVerifier);
    let runtime = profile.runtime.clone();

    let (mut client, server) = duplex(8192);
    tokio::spawn(serve_connection(server, runtime.clone()));
    attach(&mut client).await;
    confirm_over(&mut client, &profile, plan_digest(), true).await;
    let reply: PromptProviderProvisionFrame = read_json_frame(&mut client).await.unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Completed,
            ..
        }
    ));

    // Persist-before-broadcast: the durable lock already exists by the time the wire reply lands.
    assert!(
        profile
            .ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_some()
    );

    write_frame(&mut client, &WireFrame::Subscribe { after_cursor: 0 })
        .await
        .unwrap();
    let WireFrame::Events { page } = read_frame(&mut client).await.unwrap() else {
        panic!("expected an events page");
    };
    let installed = page
        .events
        .iter()
        .find(|event| event.kind == "privatePromptProvisionInstalled")
        .expect("installed event must be durable");
    let seen_cursor = installed.cursor;
    drop(client);

    // Reconnect and resume strictly after the cursor already seen: no duplicate replay.
    let (mut reconnected, server) = duplex(8192);
    tokio::spawn(serve_connection(server, runtime.clone()));
    attach(&mut reconnected).await;
    write_frame(
        &mut reconnected,
        &WireFrame::Subscribe {
            after_cursor: seen_cursor,
        },
    )
    .await
    .unwrap();
    let WireFrame::Events { page } = read_frame(&mut reconnected).await.unwrap() else {
        panic!("expected an events page");
    };
    assert!(
        page.events
            .iter()
            .all(|event| event.kind != "privatePromptProvisionInstalled")
    );

    // A fresh connection replaying from zero still sees the exact same durable fact.
    write_frame(&mut reconnected, &WireFrame::Subscribe { after_cursor: 0 })
        .await
        .unwrap();
    let WireFrame::Events { page } = read_frame(&mut reconnected).await.unwrap() else {
        panic!("expected an events page");
    };
    assert!(page.events.iter().any(
        |event| event.kind == "privatePromptProvisionInstalled" && event.cursor == seen_cursor
    ));
}

#[tokio::test]
async fn retrying_the_same_confirm_command_is_idempotent_and_never_repeats_the_npm_effect() {
    let directory = tempfile::tempdir().unwrap();
    let profile = compose(directory.path(), FakeVerifier);
    let runtime = profile.runtime.clone();

    let (mut client, server) = duplex(8192);
    tokio::spawn(serve_connection(server, runtime.clone()));
    attach(&mut client).await;
    confirm_over(&mut client, &profile, plan_digest(), true).await;
    let first: PromptProviderProvisionFrame = read_json_frame(&mut client).await.unwrap();

    confirm_over(&mut client, &profile, plan_digest(), true).await;
    let second: PromptProviderProvisionFrame = read_json_frame(&mut client).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(*profile.installer_calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn consent_refusal_settles_terminally_without_reaching_the_installer() {
    let directory = tempfile::tempdir().unwrap();
    let profile = compose(directory.path(), FakeVerifier);
    let runtime = profile.runtime.clone();

    let (mut client, server) = duplex(8192);
    tokio::spawn(serve_connection(server, runtime));
    attach(&mut client).await;
    confirm_over(&mut client, &profile, plan_digest(), false).await;
    let reply: PromptProviderProvisionFrame = read_json_frame(&mut client).await.unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::ConsentRequired,
            ..
        }
    ));
    assert_eq!(*profile.installer_calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn ambiguous_post_install_verification_settles_unprovable_over_the_wire() {
    let directory = tempfile::tempdir().unwrap();
    let profile = compose(directory.path(), AmbiguousVerifier);
    let runtime = profile.runtime.clone();

    let (mut client, server) = duplex(8192);
    tokio::spawn(serve_connection(server, runtime));
    attach(&mut client).await;
    confirm_over(&mut client, &profile, plan_digest(), true).await;
    let reply: PromptProviderProvisionFrame = read_json_frame(&mut client).await.unwrap();
    assert!(matches!(
        reply,
        PromptProviderProvisionFrame::Result {
            state: PromptProviderProvisionState::Unprovable,
            ..
        }
    ));
    assert_eq!(*profile.installer_calls.lock().unwrap(), 1);
    assert!(
        profile
            .ledger
            .find_provisioned_provider_installation("codex")
            .unwrap()
            .is_none()
    );
}
