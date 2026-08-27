//! Daemon mapping for asking, cancelling, and reading bounded side questions.
//!
//! Answering itself never happens here: `AskSideQuestion` only performs the fast, durable
//! `Pending` write and returns. The caller (see `runtime_facade_api`) is responsible for
//! dispatching the actual provider call off the request path once this returns a `Pending`
//! record — see `agent_chat_side_question_worker`.

use gent_ports::AgentChatSideQuestionLedger;
use gent_protocol::AgentChatSideQuestionFrame;
use gent_runtime::{
    AgentChatSideQuestionAskResult, AgentChatSideQuestionCancelResult, AgentChatSideQuestionService,
};
use gent_types::{
    AgentChatConversationId, AgentChatSideQuestion, AgentChatSideQuestionCancel, HostEpoch,
};

pub(crate) fn exchange<L>(
    service: &AgentChatSideQuestionService<L>,
    host_epoch: HostEpoch,
    frame: AgentChatSideQuestionFrame,
) -> Result<AgentChatSideQuestionFrame, String>
where
    L: AgentChatSideQuestionLedger,
{
    match frame {
        AgentChatSideQuestionFrame::AskSideQuestion {
            request_id,
            receipt_id,
            conversation_id,
            question,
        } => match service
            .ask(&AgentChatSideQuestion {
                request_id: gent_types::AgentChatRequestId(request_id.clone()),
                receipt_id: gent_types::ReceiptId(receipt_id),
                host_epoch,
                conversation_id: AgentChatConversationId(conversation_id),
                question,
                created_at_unix_ms: unix_millis(),
            })
            .map_err(|error| error.to_string())?
        {
            AgentChatSideQuestionAskResult::Asked(asked) => Ok(AgentChatSideQuestionFrame::Asked {
                request_id,
                record: asked.record,
            }),
            AgentChatSideQuestionAskResult::DeniedObserver => {
                Err("agent-chat authority is disabled".into())
            }
        },
        AgentChatSideQuestionFrame::CancelSideQuestion {
            request_id,
            receipt_id,
            side_question_id,
        } => match service
            .cancel(&AgentChatSideQuestionCancel {
                request_id: gent_types::AgentChatRequestId(request_id.clone()),
                receipt_id: gent_types::ReceiptId(receipt_id),
                host_epoch,
                side_question_id,
            })
            .map_err(|error| error.to_string())?
        {
            AgentChatSideQuestionCancelResult::Cancelled(cancelled) => {
                Ok(AgentChatSideQuestionFrame::Cancelled {
                    request_id,
                    record: cancelled.record,
                })
            }
            AgentChatSideQuestionCancelResult::DeniedObserver => {
                Err("agent-chat authority is disabled".into())
            }
        },
        AgentChatSideQuestionFrame::ListSideQuestions {
            request_id,
            conversation_id,
        } => service
            .list(&AgentChatConversationId(conversation_id))
            .map(|side_questions| AgentChatSideQuestionFrame::SideQuestions {
                request_id,
                side_questions,
            })
            .map_err(|error| error.to_string()),
        AgentChatSideQuestionFrame::Asked { .. }
        | AgentChatSideQuestionFrame::Cancelled { .. }
        | AgentChatSideQuestionFrame::SideQuestions { .. } => {
            Err("side question response frames are server-only".into())
        }
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
