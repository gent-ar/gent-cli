//! Receipt-backed persistence of user chat prompts without provider activation.
use std::sync::{Arc, Mutex};

use gent_ports::{ConversationPromptLedger, ConversationPromptSave, Ledger, ReceiptClaim};
use gent_types::{
    Command, ConversationMessage, ConversationPrompt, Event, HostEpoch, Receipt, ReceiptId,
    ReceiptStatus,
};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

/// Explicit receipt and prompt identity for one user-authored conversation message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationPromptRequest {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub prompt: ConversationPrompt,
}

/// Terminal outcome of the narrow conversation-prompt service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationPromptState {
    DeniedObserver,
    Saved,
    Rejected,
    Unprovable,
}

/// Receipt result. Prompt text remains in the dedicated content ledger, never an event payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationPromptResult {
    pub state: ConversationPromptState,
    pub receipt: Option<Receipt>,
    pub message: Option<ConversationMessage>,
}

/// Serializes prompt persistence after receipt ownership while observer mode performs no writes.
#[derive(Clone, Debug)]
pub struct ConversationPromptService<L> {
    ledger: L,
    authority: bool,
    serial: Arc<Mutex<()>>,
}

impl<L> ConversationPromptService<L> {
    /// Creates a service. `authority = false` returns before receipt or content persistence.
    #[must_use]
    pub fn new(ledger: L, authority: bool) -> Self {
        Self {
            ledger,
            authority,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger + ConversationPromptLedger> ConversationPromptService<L> {
    /// Saves one user prompt and its active turn. A recovered accepted receipt safely retries
    /// only this idempotent database transaction; it never starts a provider or bridge.
    ///
    /// # Errors
    /// Returns an error when receipt or durable prompt persistence cannot respond.
    pub fn submit(
        &self,
        request: &ConversationPromptRequest,
    ) -> Result<ConversationPromptResult, RuntimeError> {
        if !self.authority {
            return Ok(ConversationPromptResult {
                state: ConversationPromptState::DeniedObserver,
                receipt: None,
                message: None,
            });
        }
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let command = command_for(request);
        match self
            .ledger
            .claim_command(&command, &accepted_event(&command))?
        {
            ReceiptClaim::Accepted(receipt) => self.save(request, &receipt),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                self.save(request, &receipt)
            }
            ReceiptClaim::Existing(receipt) => self.existing(request, receipt),
        }
    }

    fn save(
        &self,
        request: &ConversationPromptRequest,
        receipt: &Receipt,
    ) -> Result<ConversationPromptResult, RuntimeError> {
        let Ok(
            ConversationPromptSave::Created(message) | ConversationPromptSave::Existing(message),
        ) = self.ledger.save_conversation_prompt(&request.prompt)
        else {
            return self.settle(receipt, ConversationPromptState::Rejected, None);
        };
        self.settle(receipt, ConversationPromptState::Saved, Some(message))
    }

    fn existing(
        &self,
        request: &ConversationPromptRequest,
        receipt: Receipt,
    ) -> Result<ConversationPromptResult, RuntimeError> {
        let message = self
            .ledger
            .find_conversation_message(&request.prompt.message_id)?;
        let state = if receipt.status == ReceiptStatus::Settled && message.is_some() {
            ConversationPromptState::Saved
        } else {
            ConversationPromptState::Unprovable
        };
        Ok(ConversationPromptResult {
            state,
            receipt: Some(receipt),
            message,
        })
    }

    fn settle(
        &self,
        receipt: &Receipt,
        state: ConversationPromptState,
        message: Option<ConversationMessage>,
    ) -> Result<ConversationPromptResult, RuntimeError> {
        let terminal = Event {
            cursor: 0,
            event_id: terminal_event_id(&receipt.receipt_id),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(state).into(),
            payload: message_payload(message.as_ref()),
        };
        let receipt = self.ledger.settle_receipt(
            &receipt.idempotency_key,
            receipt_status(state),
            &terminal,
        )?;
        Ok(ConversationPromptResult {
            state,
            receipt: Some(receipt),
            message,
        })
    }
}

fn command_for(request: &ConversationPromptRequest) -> Command {
    let text_digest_sha256 = digest(&request.prompt.text);
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "conversationPrompt".into(),
        payload: serde_json::json!({
            "messageId": request.prompt.message_id, "turnId": request.prompt.turn_id,
            "conversationId": request.prompt.conversation_id, "runId": request.prompt.run_id,
            "textDigestSha256": text_digest_sha256, "textByteLen": request.prompt.text.len(),
        }),
    }
}

fn accepted_event(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:conversation-prompt-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "conversationPromptAccepted".into(),
        payload: command.payload.clone(),
    }
}

fn terminal_event_id(receipt_id: &ReceiptId) -> String {
    format!("{}:conversation-prompt-terminal", receipt_id.0)
}

const fn receipt_status(state: ConversationPromptState) -> ReceiptStatus {
    match state {
        ConversationPromptState::Saved => ReceiptStatus::Settled,
        ConversationPromptState::Unprovable => ReceiptStatus::Unprovable,
        ConversationPromptState::DeniedObserver | ConversationPromptState::Rejected => {
            ReceiptStatus::Rejected
        }
    }
}

const fn terminal_kind(state: ConversationPromptState) -> &'static str {
    match state {
        ConversationPromptState::Saved => "conversationPromptSaved",
        ConversationPromptState::Unprovable => "conversationPromptUnprovable",
        ConversationPromptState::DeniedObserver => "conversationPromptDeniedObserver",
        ConversationPromptState::Rejected => "conversationPromptRejected",
    }
}

fn message_payload(message: Option<&ConversationMessage>) -> serde_json::Value {
    message.map_or_else(|| serde_json::json!({}), |message| serde_json::json!({
        "messageId": message.message_id, "turnId": message.turn_id, "sequence": message.sequence,
        "textDigestSha256": message.text_digest_sha256, "textByteLen": message.text.len(),
    }))
}

fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}
