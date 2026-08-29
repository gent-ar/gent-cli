//! Negotiated local IPC for asking, cancelling, and reading bounded side questions.
//!
//! Answering runs off the request path: `Asked` returns immediately with a `Pending` record,
//! and the settled `Answered`/`Failed`/`Cancelled` record is delivered later, either through a
//! `ListSideQuestions` read or through the negotiated `event-stream-v1` capability.

use gent_types::AgentChatSideQuestionRecord;
use serde::{Deserialize, Serialize};

/// Negotiated capability for asking, cancelling, and reading local side questions.
pub const AGENT_CHAT_SIDE_QUESTION_CAPABILITY: &str = "agent-chat-side-question-v1";

/// One finite side-question exchange. Provider execution is never part of this protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatSideQuestionFrame {
    AskSideQuestion {
        request_id: String,
        receipt_id: String,
        conversation_id: String,
        question: String,
    },
    Asked {
        request_id: String,
        record: AgentChatSideQuestionRecord,
    },
    CancelSideQuestion {
        request_id: String,
        receipt_id: String,
        side_question_id: String,
    },
    Cancelled {
        request_id: String,
        record: AgentChatSideQuestionRecord,
    },
    ListSideQuestions {
        request_id: String,
        conversation_id: String,
    },
    SideQuestions {
        request_id: String,
        side_questions: Vec<AgentChatSideQuestionRecord>,
    },
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_SIDE_QUESTION_CAPABILITY, AgentChatSideQuestionFrame};
    use serde_json::json;

    #[test]
    fn ask_frame_round_trips_through_camel_case_json() {
        let frame = AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: "request-1".into(),
            receipt_id: "receipt-1".into(),
            conversation_id: "conversation-1".into(),
            question: "why did this fail?".into(),
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(
            value,
            json!({
                "type": "askSideQuestion",
                "body": {
                    "requestId": "request-1", "receiptId": "receipt-1",
                    "conversationId": "conversation-1", "question": "why did this fail?"
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<AgentChatSideQuestionFrame>(value).unwrap(),
            frame
        );
        assert_eq!(
            AGENT_CHAT_SIDE_QUESTION_CAPABILITY,
            "agent-chat-side-question-v1"
        );
    }

    #[test]
    fn frame_rejects_unknown_fields() {
        let frame = json!({
            "type": "listSideQuestions",
            "body": { "requestId": "request-1", "conversationId": "conversation-1", "extra": true }
        });
        assert!(serde_json::from_value::<AgentChatSideQuestionFrame>(frame).is_err());
    }
}
