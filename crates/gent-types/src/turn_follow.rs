//! Provider-neutral terminal result for one durable conversation turn.

use serde::{Deserialize, Serialize};

use crate::DurableTurnPhase;

/// A turn-scoped completion observed after its normalized transcript is settled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnTerminal {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub phase: DurableTurnPhase,
    pub cursor: u64,
}

impl TurnTerminal {
    /// Rejects nonterminal or incomplete public identities.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.phase.is_terminal()
            && [&self.conversation_id, &self.run_id, &self.turn_id]
                .into_iter()
                .all(|value| !value.trim().is_empty() && !value.contains('\0'))
    }
}

#[cfg(test)]
mod tests {
    use super::TurnTerminal;
    use crate::DurableTurnPhase;

    #[test]
    fn terminal_requires_a_terminal_phase_and_public_ids() {
        let terminal = TurnTerminal {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            phase: DurableTurnPhase::Completed,
            cursor: 7,
        };
        assert!(terminal.is_valid());
        assert!(
            !TurnTerminal {
                phase: DurableTurnPhase::Active,
                ..terminal.clone()
            }
            .is_valid()
        );
        assert!(
            !TurnTerminal {
                turn_id: "\0".into(),
                ..terminal
            }
            .is_valid()
        );
    }
}
