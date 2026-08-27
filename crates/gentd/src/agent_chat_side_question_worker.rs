//! Off-request-path execution of one already-accepted side question's provider call.
//!
//! Every provider's side-question call blocks its own thread for up to tens of seconds (see
//! `claude_summary_runner`/`codex_summary_runner`/`claurst_summary_runner`), so this mirrors the
//! codebase's existing pattern for that shape of work (see
//! `ordinary_lifecycle_cadence_wake::OrdinaryPromptWake::schedule`): a dedicated
//! `std::thread::spawn`, not an async task, so it never occupies a shared tokio worker thread.
//! The durable settlement write is the source of truth; the appended event is a best-effort
//! nudge for a client already subscribed to `event-stream-v1` and is never required for
//! correctness — a client can always also read `ListSideQuestions`.

use gent_ports::Ledger;
use gent_runtime::AgentChatSideQuestionService;
use gent_store::SqliteLedger;
use gent_types::{Event, HostEpoch, ReceiptId};

use crate::agent_chat_side_question_runners::AgentChatSideQuestionRunnerSources;

pub(crate) const SIDE_QUESTION_ANSWERED_EVENT_KIND: &str = "agentChatSideQuestionAnswered";

pub(crate) fn dispatch(
    service: AgentChatSideQuestionService<SqliteLedger>,
    runners: Option<AgentChatSideQuestionRunnerSources>,
    ledger: SqliteLedger,
    host_epoch: HostEpoch,
    side_question_id: String,
    conversation_id: String,
    question: String,
) {
    std::thread::spawn(move || {
        let settled = service.run_and_complete(
            &side_question_id,
            &conversation_id,
            &question,
            |provider, workspace_path| {
                runners.as_ref().map_or_else(
                    || {
                        Err(gent_ports::PortError::Unavailable(
                            "side questions are unavailable for this runtime".into(),
                        ))
                    },
                    |runners| runners.resolve(provider, workspace_path),
                )
            },
        );
        let Ok(record) = settled else {
            return;
        };
        let _ = ledger.append_event(&Event {
            cursor: 0,
            event_id: format!("agent-chat-side-question-answered-{side_question_id}"),
            receipt_id: ReceiptId(side_question_id),
            host_epoch,
            kind: SIDE_QUESTION_ANSWERED_EVENT_KIND.into(),
            payload: serde_json::json!({
                "sideQuestionId": record.side_question_id,
                "conversationId": record.conversation_id.0,
                "status": record.status,
            }),
        });
    });
}
