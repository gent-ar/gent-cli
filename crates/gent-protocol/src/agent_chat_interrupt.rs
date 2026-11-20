use gent_types::DurableTurnPhase;
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_INTERRUPT_TURN_ENDED_CAPABILITY: &str = "agent-chat-interrupt-turn-ended-v1";
pub const INTERRUPT_TURN_ENDED: &str = "interruptTurnEnded";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterruptTurnEnded {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub turn_phase: DurableTurnPhase,
    pub background_work_running: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InterruptRejectionFrame {
    Error {
        code: String,
        message: String,
        interrupt: InterruptTurnEnded,
    },
}

#[cfg(test)]
mod tests {
    use super::{INTERRUPT_TURN_ENDED, InterruptRejectionFrame, InterruptTurnEnded};
    use gent_types::DurableTurnPhase;
    use serde_json::json;

    #[test]
    fn an_ended_turn_rejection_names_the_turn_and_its_background_work() {
        let frame = InterruptRejectionFrame::Error {
            code: INTERRUPT_TURN_ENDED.into(),
            message: "the turn already ended".into(),
            interrupt: InterruptTurnEnded {
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                turn_id: "turn".into(),
                turn_phase: DurableTurnPhase::Completed,
                background_work_running: true,
            },
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({"type": "error", "body": {
                "code": "interruptTurnEnded",
                "message": "the turn already ended",
                "interrupt": {
                    "conversationId": "conversation",
                    "runId": "run",
                    "turnId": "turn",
                    "turnPhase": "completed",
                    "backgroundWorkRunning": true
                }
            }})
        );
    }
}
