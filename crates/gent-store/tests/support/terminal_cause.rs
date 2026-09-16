use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptSaved, ConversationActivityFact, TurnTerminalCause};

#[derive(Debug, Eq, PartialEq)]
pub enum RecordedTerminal {
    Absent,
    Untyped,
    Caused(TurnTerminalCause),
}

pub fn terminal_cause(ledger: &SqliteLedger, prompt: &AgentChatPromptSaved) -> RecordedTerminal {
    use gent_ports::ConversationActivityLedger as _;
    ledger
        .read_conversation_activity_page(&prompt.message.conversation_id, &prompt.run_id.0, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find_map(|fact| match fact {
            ConversationActivityFact::Terminal { scope, cause, .. }
                if scope.turn_id == prompt.message.turn_id =>
            {
                Some(cause.map_or(RecordedTerminal::Untyped, RecordedTerminal::Caused))
            }
            _ => None,
        })
        .unwrap_or(RecordedTerminal::Absent)
}
