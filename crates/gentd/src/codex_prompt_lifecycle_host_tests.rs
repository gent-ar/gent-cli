use gent_drivers::codex_runner::CodexRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    TranscriptLedger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatPromptCreate,
    AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId, AgentChatRunId,
    CapabilitySet, HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, ReceiptId,
    TurnPhase, WorkspaceRecord,
};

use crate::approved_codex_host::ApprovedCodexHost;
use crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome;
use crate::public_driver_runtime::PublicDriversRuntime;

use super::codex_prompt_lifecycle_tests::{
    Resolver, Runner, assert_prepared_options, profile, selection,
};

#[test]
fn codex_host_reserves_then_persists_normalized_facts_and_settles() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-a".into());
    create_conversation(&ledger, conversation_id.clone());
    let prompt = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-a".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "hello".into(),
        })
        .unwrap();
    let runner = Runner::default();
    runner.state.lock().unwrap().effects.push_back(vec![
        CodexRunnerEffect::Fact(PublicWireFact::SessionStarted {
            provider_session_id: "private-thread".into(),
        }),
        CodexRunnerEffect::Fact(PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello back".into(),
            is_partial: false,
        })),
        CodexRunnerEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Ready,
            },
        )),
    ]);
    let compatibility = super::codex_prompt_lifecycle_tests::compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedCodexHost::new(
        runtime,
        "daemon-a".into(),
        Some("/work".into()),
        HostEpoch(1),
        1,
    );
    let tick = host.tick().unwrap();
    assert_eq!(
        tick.dispatch,
        Some(CodexPromptDispatchOutcome::Started {
            run_id: "run-a".into()
        })
    );
    assert_eq!(runner.state.lock().unwrap().starts, 1);
    assert_prepared_options(&runner);
    assert_eq!(tick.polled_runs, 0);
    let tick = host.tick().unwrap();
    assert_eq!(tick.polled_runs, 1);
    assert_eq!(tick.facts, 3);
    let transcript = ledger
        .normalized_transcript_page(&conversation_id, 0, 10)
        .unwrap();
    assert_eq!(transcript.events.len(), 1);
    assert_eq!(transcript.events[0].text, "hello back");
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("prompt-b".into()),
            receipt_id: ReceiptId("prompt-receipt-b".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "follow up".into(),
        })
        .unwrap();
    assert!(matches!(
        host.tick().unwrap().dispatch,
        Some(CodexPromptDispatchOutcome::Started { .. })
    ));
    let state = runner.state.lock().unwrap();
    assert_eq!(state.starts, 1);
    assert_eq!(state.submitted, ["follow up"]);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    assert_eq!(prompt.message.text, "hello");
}

fn create_conversation(ledger: &SqliteLedger, conversation_id: AgentChatConversationId) {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id,
                run_id: AgentChatRunId("run-a".into()),
                selection: selection(),
            },
            &WorkspaceRecord {
                workspace_id: "workspace-a".into(),
                canonical_path: "/workspace-a".into(),
            },
        )
        .unwrap();
}
