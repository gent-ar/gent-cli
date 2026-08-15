use gent_ports::{
    ConversationContentReader, ConversationLedger, ConversationPromptLedger, Ledger, ReceiptClaim,
};
use gent_runtime::{
    ConversationPromptRequest, ConversationPromptService, ConversationPromptState, Coordinator,
};
use gent_store::SqliteLedger;
use gent_types::{Command, ConversationPrompt, ConversationRecord, Event, HostEpoch, ReceiptId};
use serde_json::json;

fn request(key: &str, message_id: &str, text: &str) -> ConversationPromptRequest {
    ConversationPromptRequest {
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        prompt: ConversationPrompt {
            message_id: message_id.into(),
            turn_id: format!("turn-{message_id}"),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            text: text.into(),
        },
    }
}

fn ledger_with_conversation() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "conversation".into(),
            },
            &gent_ports::RunRecord {
                run_id: "run".into(),
                parent_run_id: None,
                provider: "claude".into(),
            },
        )
        .unwrap();
    ledger
}

#[test]
fn observer_mode_never_claims_receipts_or_persists_prompt_text() {
    let ledger = ledger_with_conversation();
    let result = ConversationPromptService::new(ledger.clone(), false)
        .submit(&request("observer", "message-observer", "hello"))
        .unwrap();
    assert_eq!(result.state, ConversationPromptState::DeniedObserver);
    assert!(result.receipt.is_none());
    assert!(
        ledger
            .find_conversation_message("message-observer")
            .unwrap()
            .is_none()
    );
}

#[test]
fn authority_saves_one_prompt_turn_and_recovers_the_same_result_idempotently() {
    let ledger = ledger_with_conversation();
    let service = ConversationPromptService::new(ledger.clone(), true);
    let request = request("saved", "message-saved", "hello world");
    let first = service.submit(&request).unwrap();
    let second = service.submit(&request).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.state, ConversationPromptState::Saved);
    let message = first.message.unwrap();
    assert_eq!(message.sequence, 1);
    assert_eq!(message.text, "hello world");
    assert_eq!(ledger.list_run_messages("run").unwrap().len(), 1);
    let event = ledger
        .find_event("receipt-saved:conversation-prompt-terminal")
        .unwrap()
        .unwrap();
    assert_eq!(
        event.payload["textDigestSha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(event.payload["textByteLen"], 11);
    assert!(!event.payload.to_string().contains("hello world"));
}

#[test]
fn an_accepted_receipt_safely_replays_only_the_atomic_database_save() {
    let ledger = ledger_with_conversation();
    let request = request("restart", "message-restart", "recover me");
    let command = Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "conversationPrompt".into(),
        payload: json!({
            "messageId": "message-restart", "turnId": "turn-message-restart", "conversationId": "conversation", "runId": "run",
            "textDigestSha256": "bc54d1d8c0a99336ea2c89cccee81d1545b9e5c10791b3e5a7140803035213fb", "textByteLen": 10,
        }),
    };
    let accepted = Event {
        cursor: 0,
        event_id: "receipt-restart:conversation-prompt-accepted".into(),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "conversationPromptAccepted".into(),
        payload: command.payload.clone(),
    };
    assert!(matches!(
        ledger.claim_command(&command, &accepted).unwrap(),
        ReceiptClaim::Accepted(_)
    ));
    let result = ConversationPromptService::new(ledger.clone(), true)
        .submit(&request)
        .unwrap();
    assert_eq!(result.state, ConversationPromptState::Saved);
    assert_eq!(ledger.list_run_messages("run").unwrap().len(), 1);
}

#[test]
fn invalid_hierarchy_rejects_without_message_or_turn() {
    let ledger = ledger_with_conversation();
    let mut request = request("invalid", "message-invalid", "no write");
    request.prompt.run_id = "other-run".into();
    let result = ConversationPromptService::new(ledger.clone(), true)
        .submit(&request)
        .unwrap();
    assert_eq!(result.state, ConversationPromptState::Rejected);
    assert!(
        ledger
            .find_conversation_message("message-invalid")
            .unwrap()
            .is_none()
    );
}

#[test]
fn content_pages_are_newest_first_and_bound_to_one_conversation() {
    let ledger = ledger_with_conversation();
    let service = ConversationPromptService::new(ledger.clone(), true);
    for (key, message, text) in [
        ("one", "message-one", "first"),
        ("two", "message-two", "second"),
    ] {
        service.submit(&request(key, message, text)).unwrap();
    }
    let page = ledger
        .read_conversation_content("conversation", None, 1)
        .unwrap();
    assert_eq!(page.entries[0].text, "second");
    let cursor = page.next_before.as_ref().unwrap();
    let coordinator = Coordinator::new(ledger, gent_types::CapabilitySet::default());
    let next = coordinator
        .conversation_content("conversation", Some(cursor), 1)
        .unwrap();
    assert_eq!(next.entries[0].text, "first");
    assert!(next.next_before.is_none());
    assert!(
        coordinator
            .conversation_content("other", Some(cursor), 1)
            .is_err()
    );
}

#[test]
fn content_page_is_byte_bounded_without_skipping_messages() {
    let ledger = ledger_with_conversation();
    let service = ConversationPromptService::new(ledger.clone(), true);
    for index in 0..5 {
        let key = format!("large-{index}");
        let message_id = format!("message-large-{index}");
        let text = "x".repeat(64 * 1024);
        service.submit(&request(&key, &message_id, &text)).unwrap();
    }
    let first = ledger
        .read_conversation_content("conversation", None, 100)
        .unwrap();
    assert!(serde_json::to_vec(&first).unwrap().len() <= 256 * 1024);
    assert_eq!(first.entries.len(), 3);
    let next = ledger
        .read_conversation_content(
            "conversation",
            first
                .next_before
                .as_ref()
                .map(|cursor| cursor.ordinal_for("conversation").unwrap()),
            100,
        )
        .unwrap();
    assert_eq!(next.entries.len(), 2);
    assert!(next.next_before.is_none());
}
