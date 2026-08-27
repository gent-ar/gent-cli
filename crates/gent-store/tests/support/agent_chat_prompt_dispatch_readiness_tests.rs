use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger, TranscriptLedger};
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, HostEpoch, ReceiptId,
};

use super::{ledger, prompt};

#[test]
fn prelaunch_failure_settles_the_claimed_turn_and_leaves_later_prompts_claimable() {
    let (ledger, conversation_id) = ledger();
    let failed = prompt(
        &ledger,
        &conversation_id,
        "failed",
        AgentChatPromptDisposition::Send,
    );
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    ledger
        .fail_agent_chat_prompt_prelaunch(
            &claimed.message.message_id,
            "daemon-a",
            HostEpoch(1),
            "local Claurst startup exceeded 45 seconds",
        )
        .unwrap();
    let page = ledger
        .normalized_transcript_page(&conversation_id, 0, 10)
        .unwrap();
    assert!(
        page.events
            .iter()
            .any(|event| event.turn_id == failed.message.turn_id
                && event.text.contains("startup exceeded"))
    );
    let later = prompt(
        &ledger,
        &conversation_id,
        "later",
        AgentChatPromptDisposition::Send,
    );
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .unwrap()
            .message,
        later.message
    );
}

#[test]
fn unreviewed_send_prompt_is_not_claimable() {
    let (ledger, conversation_id) = ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("held".into()),
            receipt_id: ReceiptId("receipt-held".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "held until provider readiness".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            HostEpoch(1),
        )
        .unwrap();
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .unwrap()
            .message,
        saved.message
    );
}

#[test]
fn readiness_release_rejects_another_run_without_exposing_outbox_work() {
    let (ledger, conversation_id) = ledger();
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId("wrong-run".into()),
            receipt_id: ReceiptId("receipt-wrong-run".into()),
            host_epoch: HostEpoch(1),
            conversation_id,
            disposition: AgentChatPromptDisposition::Send,
            text: "must retain exact reviewed run".into(),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    assert!(
        ledger
            .release_agent_chat_prompt_after_readiness(
                &saved.message.message_id,
                &AgentChatRunId("other-run".into()),
                HostEpoch(1),
            )
            .is_err()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}
