use gent_ports::{
    ClaurstCheckpoint, ClaurstFactValue, ClaurstSessionBinding, ClaurstTerminal, PortError,
};
use sha2::{Digest, Sha256};

use crate::claurst_acp_transport::{ClaurstAcpFact, ClaurstAcpTerminal, ClaurstAcpTransportError};

pub(super) fn project(fact: ClaurstAcpFact) -> ClaurstFactValue {
    match fact {
        ClaurstAcpFact::Event(event) => ClaurstFactValue::Event(event),
        ClaurstAcpFact::Lifecycle(signal) => ClaurstFactValue::Lifecycle(signal),
    }
}
pub(super) fn project_terminal(terminal: ClaurstAcpTerminal) -> ClaurstTerminal {
    match terminal {
        ClaurstAcpTerminal::Completed => ClaurstTerminal::Completed,
        ClaurstAcpTerminal::Interrupted => ClaurstTerminal::Interrupted,
        ClaurstAcpTerminal::Failed => ClaurstTerminal::Failed {
            classification: gent_ports::ClaurstFailureClassification::Protocol,
        },
    }
}
pub(super) fn checkpoint(binding: &ClaurstSessionBinding, cursor: u64) -> ClaurstCheckpoint {
    let digest = format!(
        "{}\0{}\0{}\0{cursor}",
        binding.run_id, binding.source_id.0, binding.opaque_session_id
    );
    ClaurstCheckpoint {
        run_id: binding.run_id.clone(),
        source_id: binding.source_id.clone(),
        cursor,
        state_digest_sha256: format!("{:x}", Sha256::digest(digest.as_bytes())),
    }
}
pub(super) fn provider(error: ClaurstAcpTransportError) -> PortError {
    PortError::Provider(error.to_string())
}
pub(super) fn invalid(what: &str) -> PortError {
    PortError::Provider(format!("invalid Claurst ACP {what}"))
}
pub(super) fn unavailable(what: &str) -> PortError {
    PortError::Unavailable(what.into())
}
