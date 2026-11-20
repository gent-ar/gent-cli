use gent_ports::LedgerError;
use gent_types::{ConversationActivityFact, HostEpoch, ModelDownloadPhase, ReceiptId};
use rusqlite::TransactionBehavior;

use super::super::SqliteLedger;
use super::super::queries::{find_event, storage_error};
use super::prompt_admission_hold::{held_send_prompt, scope};
use super::prompt_dispatch::require_open;
use crate::sqlite::agent_chat_queue_activity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelDownloadReport<'a> {
    pub model_id: &'a str,
    pub phase: ModelDownloadPhase,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

impl SqliteLedger {
    pub fn record_model_download_progress(
        &self,
        prompt_receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        report: ModelDownloadReport<'_>,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        require_open(&transaction, host_epoch)?;
        let Some(prompt) = held_send_prompt(&transaction, prompt_receipt_id)? else {
            return Ok(());
        };
        let event_id = format!(
            "agent-chat-model-download:{}:{:?}:{}",
            prompt.message, report.phase, report.downloaded_bytes
        );
        if find_event(&transaction, &event_id)?.is_some() {
            return Ok(());
        }
        agent_chat_queue_activity::append(
            &transaction,
            event_id,
            ReceiptId(format!("agentChatModelDownload:{}", prompt.message)),
            ConversationActivityFact::ModelDownloadProgress {
                scope: scope(&prompt, host_epoch),
                model_id: report.model_id.to_owned(),
                message_id: prompt.message.clone(),
                receipt_id: prompt_receipt_id.0.clone(),
                downloaded_bytes: report.downloaded_bytes,
                total_bytes: report.total_bytes,
                phase: report.phase,
            },
        )?;
        transaction.commit().map_err(storage_error)
    }
}
