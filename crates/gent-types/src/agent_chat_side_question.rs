//! Typed request/result for a bounded, provider-neutral question about a conversation.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, AgentChatRequestId, HostEpoch, Receipt, ReceiptId};

/// Client correlation and question text required to ask a side question about a conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSideQuestion {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub question: String,
    pub created_at_unix_ms: u64,
}

/// Client correlation required to cancel a side question that has not yet answered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSideQuestionCancel {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub side_question_id: String,
}

/// Durable lifecycle of one side question, from acceptance through its final outcome.
///
/// Cancellation is a durable record only: it stops a client from waiting on the answer, but it
/// does not interrupt an already-dispatched provider process. See
/// [`AgentChatSideQuestionRecord`] for the terminal text this status accompanies.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentChatSideQuestionStatus {
    Pending,
    Answered,
    Failed,
    Cancelled,
}

/// A retry-stable side question, its status, and its answer once settled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatSideQuestionRecord {
    pub side_question_id: String,
    pub conversation_id: AgentChatConversationId,
    pub question: String,
    pub status: AgentChatSideQuestionStatus,
    pub answer: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at_unix_ms: u64,
}

/// The accepted, durable acknowledgement of a newly asked side question.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSideQuestionAsked {
    pub receipt: Receipt,
    pub record: AgentChatSideQuestionRecord,
}

/// The accepted, durable acknowledgement of a side question cancellation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatSideQuestionCancelled {
    pub receipt: Receipt,
    pub record: AgentChatSideQuestionRecord,
}

/// The outcome a background worker persists once a dispatched side question finishes running.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatSideQuestionOutcome {
    Answered { text: String },
    Failed { reason: String },
}
