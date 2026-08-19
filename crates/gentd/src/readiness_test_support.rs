//! Explicit verified-readiness fixture for dormant provider lifecycle tests.

use gent_ports::AgentChatPromptDispatchLedger;
use gent_store::SqliteLedger;
use gent_types::AgentChatPromptSaved;

/// Releases one saved prompt exactly as a future verified readiness authority would.
pub(crate) fn release(ledger: &SqliteLedger, saved: &AgentChatPromptSaved) {
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            saved.receipt.host_epoch,
        )
        .unwrap();
}
