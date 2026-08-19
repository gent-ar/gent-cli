use gent_ports::{AgentChatLedger, ConversationLedger};
use gent_runtime::TurnFollowRequest;
use gent_runtime::catalog::declared_capabilities;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    TurnRecord,
};

use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
};

#[test]
fn preopened_composition_state_builds_the_identical_observer_facade() {
    let directory = tempfile::tempdir().unwrap();
    let capabilities = declared_capabilities();
    let state = DaemonCompositionState::open(
        directory.path(),
        &capabilities,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert_eq!(state.data_dir(), directory.path());
    assert_eq!(
        state.coordinator().status().unwrap().capabilities,
        capabilities
    );
    assert_eq!(
        state.compatibility().manifest_sha256(),
        CompatibilityAssessment::default().manifest_sha256()
    );

    let runtime = RuntimeFacade::from_state(state, None).unwrap();
    assert_eq!(runtime.capabilities().unwrap(), capabilities);
    assert_eq!(runtime.status().unwrap().capabilities, capabilities);
}

#[test]
fn future_turn_follow_authority_is_explicit_and_keeps_observer_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let capabilities = declared_capabilities();
    let state = DaemonCompositionState::open(
        directory.path(),
        &capabilities,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let ledger = state.ledger().clone();
    ledger.create_agent_chat_conversation(&create()).unwrap();
    ledger
        .create_turn(&TurnRecord {
            turn_id: "turn".into(),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            sequence: 1,
            phase: DurableTurnPhase::Completed,
        })
        .unwrap();
    let runtime = RuntimeFacade::from_state_with_turn_follow_authority(state, None).unwrap();
    let read = runtime.agent_chat_turn_follow(request()).unwrap();
    assert!(read.terminal.is_some());
    assert!(
        !runtime
            .capabilities()
            .unwrap()
            .0
            .iter()
            .any(|capability| capability == "agent-chat-turn-follow-v1")
    );

    let observer_directory = directory.path().join("observer");
    std::fs::create_dir(&observer_directory).unwrap();
    let observer = crate::build_runtime(
        &observer_directory,
        &capabilities,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(observer.agent_chat_turn_follow(request()).is_err());
}

fn create() -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: ReceiptId("receipt".into()),
        idempotency_key: "key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "model".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        },
    }
}

fn request() -> TurnFollowRequest {
    TurnFollowRequest {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        after_cursor: 0,
        expected_host_epoch: HostEpoch(1),
        limit: 100,
    }
}
