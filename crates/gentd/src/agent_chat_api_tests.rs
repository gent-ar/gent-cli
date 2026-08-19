use std::cell::Cell;

use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AgentChatSelectionSwitchAuthority, AgentChatSelectionSwitchService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    ReceiptId,
};

use crate::agent_chat_api::{PromptCommitWake, PromptWake, exchange, exchange_with_wake};

struct Wake {
    calls: Cell<u8>,
    last: Option<PromptWake>,
    failure: Option<&'static str>,
}

impl Wake {
    const fn available() -> Self {
        Self {
            calls: Cell::new(0),
            last: None,
            failure: None,
        }
    }
}

impl PromptCommitWake for Wake {
    type Error = &'static str;

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.last = Some(prompt);
        self.failure.map_or(Ok(()), Err)
    }
}

#[test]
fn generic_prompt_persistence_holds_send_without_notifying_lifecycle() {
    let (_, conversations, prompts, switches) = services();
    let created = exchange(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        create(),
    )
    .unwrap();
    let (conversation_id, root_run_id) = match &created[0] {
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        } => (conversation_id.clone(), run_id.clone()),
        _ => panic!("create must return one conversation"),
    };
    let mut wake = Wake::available();
    let accepted = exchange_with_wake(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id: conversation_id.clone(),
            text: "continue".into(),
        },
        &mut wake,
    )
    .unwrap();
    assert!(matches!(
        accepted.as_slice(),
        [AgentChatIntentFrame::Accepted { conversation_id: accepted_conversation, run_id, turn_id, .. }]
            if accepted_conversation == &conversation_id && run_id == &root_run_id && !turn_id.is_empty()
    ));
    assert_eq!(wake.calls.get(), 0);
    assert!(wake.last.is_none());
}

#[test]
fn unavailable_generic_wake_cannot_make_a_held_prompt_claimable() {
    let (ledger, conversations, prompts, switches) = services();
    let created = exchange(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        create(),
    )
    .unwrap();
    let conversation_id = match &created[0] {
        AgentChatIntentFrame::Created {
            conversation_id, ..
        } => conversation_id.clone(),
        _ => panic!("create must return one conversation"),
    };
    let mut wake = Wake {
        calls: Cell::new(0),
        last: None,
        failure: Some("bounded host unavailable"),
    };
    let accepted = exchange_with_wake(
        &conversations,
        &prompts,
        &switches,
        gent_types::HostEpoch(1),
        AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt-request".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id,
            text: "continue".into(),
        },
        &mut wake,
    )
    .unwrap();
    assert!(matches!(
        accepted.as_slice(),
        [AgentChatIntentFrame::Accepted { .. }]
    ));
    assert_eq!(wake.calls.get(), 0);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(
                "daemon-a",
                gent_types::HostEpoch(1),
                AgentChatProvider::Codex,
            )
            .unwrap()
            .is_none()
    );
}

fn services() -> (
    SqliteLedger,
    AgentChatConversationService<SqliteLedger>,
    AgentChatPromptService<SqliteLedger>,
    AgentChatSelectionSwitchService<SqliteLedger>,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    (
        ledger.clone(),
        AgentChatConversationService::new(ledger.clone(), AgentChatConversationAuthority::Approved),
        AgentChatPromptService::new(ledger.clone(), AgentChatPromptAuthority::Approved),
        AgentChatSelectionSwitchService::new(ledger, AgentChatSelectionSwitchAuthority::Approved),
    )
}

fn create() -> AgentChatIntentFrame {
    AgentChatIntentFrame::CreateConversation {
        request_id: AgentChatRequestId("create-request".into()),
        receipt_id: ReceiptId("create-receipt".into()),
        workspace_path: ".".into(),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
    }
}
