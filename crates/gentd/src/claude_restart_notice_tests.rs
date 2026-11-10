use gent_drivers::claude_runner::ClaudeRunnerEffect;
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatReadLedger, AgentChatWorkspaceLedger, Ledger,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatFailureCode,
    AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, HostEpoch,
    NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

use super::{Resolver, Runner, compatibility, profile, prompt};
use crate::approved_claude_host::ApprovedClaudeHost;
use crate::public_driver_runtime::PublicDriversRuntime;

#[test]
fn a_turn_orphaned_by_a_gentd_restart_fails_with_a_typed_notice() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-r".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation_id.clone(),
                run_id: AgentChatRunId("run-r".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace-r".into(),
                canonical_path: "/workspace-r".into(),
            },
        )
        .unwrap();
    let saved = prompt(&ledger, &conversation_id, "r1");
    let runner = Runner::default();
    runner
        .0
        .lock()
        .unwrap()
        .effects
        .push_back(vec![ClaudeRunnerEffect::Fact(
            PublicWireFact::SessionStarted {
                provider_session_id: "private-session".into(),
            },
        )]);
    let compatibility = compatibility();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner,
        Resolver,
    )
    .unwrap();
    let mut host = ApprovedClaudeHost::new(runtime, "daemon-r".into(), HostEpoch(1), 1, None);
    host.tick().unwrap();
    host.tick().unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    let notices = ledger
        .read_agent_chat_transcript(&conversation_id.0, None, 50)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            event.kind == NormalizedTranscriptKind::Notice && event.turn_id == saved.message.turn_id
        })
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].failure_code,
        Some(AgentChatFailureCode::GentdRestarted)
    );
    assert_eq!(
        notices[0].text,
        "Gentd restarted while this turn was running. Send the prompt again to continue."
    );
}
