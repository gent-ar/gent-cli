use async_trait::async_trait;
use gent_store::SqliteLedger;

use super::{OrdinaryPromptIngress, PromptCommitWake, PromptWake};

#[async_trait]
pub(super) trait StandaloneHeldRecovery: Send + Sync + std::fmt::Debug {
    async fn recover(&self) -> Result<(), String>;
}

#[async_trait]
impl StandaloneHeldRecovery for OrdinaryPromptIngress<SqliteLedger> {
    async fn recover(&self) -> Result<(), String> {
        let Some(readiness) = &self.standalone_readiness else {
            return Ok(());
        };
        let mut after = String::new();
        loop {
            let held = readiness.clone();
            let cursor = after.clone();
            let page = tokio::task::spawn_blocking(move || held.awaiting_prompts(&cursor))
                .await
                .map_err(|_| "held prompt recovery read failed")??;
            if page.is_empty() {
                return Ok(());
            }
            for prompt in page {
                after.clone_from(&prompt.message.message_id);
                let mut ingress = self.clone();
                ingress.wake_after_prompt_commit(PromptWake {
                    conversation_id: gent_types::AgentChatConversationId(
                        prompt.message.conversation_id,
                    ),
                    run_id: prompt.run_id,
                    receipt_id: prompt.receipt.receipt_id,
                    disposition: prompt.disposition,
                })?;
            }
        }
    }
}
