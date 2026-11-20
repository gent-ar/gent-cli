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
    asked: gent_types::AgentChatSideQuestionRecord,
    operation: Option<crate::ordinary_lifecycle_control::OrdinaryPromptPermit>,
) {
    std::thread::spawn(move || {
        let _operation = operation;
        let side_question_id = asked.side_question_id;
        let settled = service.run_and_complete(
            &side_question_id,
            &asked.conversation_id.0,
            &asked.question,
            |provider, workspace_path| {
                runners.as_ref().map_or_else(
                    || {
                        Err(gent_ports::PortError::Unavailable(
                            "side questions are unavailable for this runtime".into(),
                        ))
                    },
                    |runners| runners.resolve(provider, workspace_path, &asked.conversation_id.0),
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
