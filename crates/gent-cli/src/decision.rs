//! Decision command parsing and frame construction; socket orchestration stays in `main`.

use clap::Subcommand;
use gent_protocol::{DecisionRecoveryEvidence, WireFrame};
use gent_types::DecisionCommand;

/// Explicit client actions over one durable decision record.
#[derive(Debug, Subcommand)]
pub enum DecisionCommandLine {
    /// Persist a decision idempotently before any provider observes it.
    Submit {
        #[arg(long)]
        decision_id: String,
        #[arg(long)]
        idempotency_key: String,
    },
    /// Terminally record that provider acknowledgement cannot be proven.
    Unprovable {
        #[arg(long)]
        decision_id: String,
    },
    /// Terminally require recovery when the original decision cannot safely continue.
    Recovery {
        #[arg(long)]
        decision_id: String,
    },
}

/// Converts an explicit CLI action into a protocol DTO without touching the daemon or store.
#[must_use]
pub fn decision_frame(action: &DecisionCommandLine) -> WireFrame {
    match action {
        DecisionCommandLine::Submit {
            decision_id,
            idempotency_key,
        } => WireFrame::DecisionSubmit(DecisionCommand {
            decision_id: decision_id.clone(),
            idempotency_key: idempotency_key.clone(),
        }),
        DecisionCommandLine::Unprovable { decision_id } => recovery(
            decision_id,
            DecisionRecoveryEvidence::AcknowledgementUnprovable,
        ),
        DecisionCommandLine::Recovery { decision_id } => {
            recovery(decision_id, DecisionRecoveryEvidence::RecoveryRequired)
        }
    }
}

fn recovery(decision_id: &str, evidence: DecisionRecoveryEvidence) -> WireFrame {
    WireFrame::DecisionRecovery {
        decision_id: decision_id.into(),
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use gent_protocol::{DecisionRecoveryEvidence, WireFrame};

    use super::{DecisionCommandLine, decision_frame};

    #[test]
    fn submit_is_typed_and_idempotency_key_is_not_synthesized() {
        assert!(matches!(
            decision_frame(&DecisionCommandLine::Submit {
                decision_id: "ask-1".into(),
                idempotency_key: "key-1".into(),
            }),
            WireFrame::DecisionSubmit(command)
                if command.decision_id == "ask-1" && command.idempotency_key == "key-1"
        ));
    }

    #[test]
    fn terminal_actions_are_explicit_protocol_evidence() {
        assert!(matches!(
            decision_frame(&DecisionCommandLine::Unprovable {
                decision_id: "ask-1".into(),
            }),
            WireFrame::DecisionRecovery {
                evidence: DecisionRecoveryEvidence::AcknowledgementUnprovable,
                ..
            }
        ));
        assert!(matches!(
            decision_frame(&DecisionCommandLine::Recovery {
                decision_id: "ask-1".into(),
            }),
            WireFrame::DecisionRecovery {
                evidence: DecisionRecoveryEvidence::RecoveryRequired,
                ..
            }
        ));
    }
}
