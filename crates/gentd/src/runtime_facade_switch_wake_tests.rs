use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger};
use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatProvider, AgentChatRequestId, AgentChatSelection, ContextPolicy, HostEpoch, ReceiptId,
};

use super::{facade_with_hosts, root};
use crate::{api::RuntimeApi, ordinary_lifecycle_router::OrdinaryLifecycleHost};

#[derive(Debug)]
struct Claude(Arc<AtomicUsize>);

impl OrdinaryLifecycleHost for Claude {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claude
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn wake(&mut self) -> Result<(), ()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn drive(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        false
    }
    fn begin_shutdown_after_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn shutdown_complete(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_prompt_waiting_for_claurst_wakes_claude_when_a_switch_carries_it_there() {
    let directory = tempfile::tempdir().unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let runtime =
        facade_with_hosts(directory.path(), vec![Box::new(Claude(Arc::clone(&wakes)))]).await;
    let conversation = root(&runtime, "carried", ".");
    let ledger = runtime.conversation_links().unwrap().clone();
    let waiting = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("waiting".into()),
            receipt_id: ReceiptId("waiting-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            disposition: AgentChatPromptDisposition::Send,
            text: "Write a numbered list of 40 distinct fruits".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    crate::readiness_test_support::release(&ledger, &waiting);
    let switched = runtime
        .agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("switch-to-claude".into()),
            receipt_id: ReceiptId("switch-to-claude-receipt".into()),
            conversation_id: conversation.clone(),
            parent_run_id: waiting.run_id.clone(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Low,
                mode: AgentChatMode::Ask,
            },
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
    let [AgentChatIntentFrame::Switched { run_id: child, .. }] = switched.as_slice() else {
        panic!("a switch with only a waiting prompt must be accepted");
    };
    assert!(wakes.load(Ordering::SeqCst) > 0);
    let carried = ledger
        .claim_agent_chat_prompt_dispatch("daemon", HostEpoch(1), AgentChatProvider::Claude)
        .unwrap()
        .unwrap();
    assert_eq!(
        (carried.message.message_id, &carried.run_id),
        (waiting.message.message_id, child)
    );
}
