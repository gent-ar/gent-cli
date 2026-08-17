//! Authority-gated persistence of daemon-normalized agent-chat transcript facts.
//!
//! This boundary accepts typed content only after a daemon-owned runner or private bridge has
//! normalized it. It deliberately has no provider protocol, process, or client-IPC dependency.

use gent_ports::TranscriptLedger;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptKind,
};

use crate::RuntimeError;

/// Explicit permission for daemon-owned normalized transcript ingress.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatTranscriptAuthority {
    /// Observer mode never inspects hierarchy or writes transcript state.
    #[default]
    Observer,
    /// Reserved for an evidence-approved daemon-owned provider composition.
    Approved,
}

/// One hierarchy-bound normalized transcript fact emitted by a daemon-owned producer.
///
/// No provider-native frame, session identity, or client-selected cursor can cross this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatTranscriptAppendRequest {
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub turn_id: String,
    pub event_id: String,
    pub kind: NormalizedTranscriptKind,
    pub text: String,
    pub is_partial: bool,
}

/// A no-write observer denial or the cursor assigned by durable storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatTranscriptAppendResult {
    DeniedObserver,
    Persisted(NormalizedTranscriptEvent),
}

/// Persists one normalized transcript fact only after explicit daemon authority approval.
#[derive(Clone, Debug)]
pub struct AgentChatTranscriptIngress<L> {
    ledger: L,
    authority: AgentChatTranscriptAuthority,
}

impl<L> AgentChatTranscriptIngress<L> {
    /// Builds an inert observer ingress unless daemon-owned authority is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatTranscriptAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: TranscriptLedger> AgentChatTranscriptIngress<L> {
    /// Appends one hierarchy-bound, provider-neutral fact through the durable transcript ledger.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches durable hierarchy validation.
    pub fn append(
        &self,
        request: &AgentChatTranscriptAppendRequest,
    ) -> Result<AgentChatTranscriptAppendResult, RuntimeError> {
        if self.authority != AgentChatTranscriptAuthority::Approved {
            return Ok(AgentChatTranscriptAppendResult::DeniedObserver);
        }
        let event = self
            .ledger
            .append_normalized_transcript(&request.conversation_id, &normalized(request))?;
        Ok(AgentChatTranscriptAppendResult::Persisted(event))
    }
}

fn normalized(request: &AgentChatTranscriptAppendRequest) -> NormalizedTranscriptAppend {
    NormalizedTranscriptAppend {
        event_id: request.event_id.clone(),
        turn_id: request.turn_id.clone(),
        run_id: request.run_id.0.clone(),
        kind: request.kind,
        text: request.text.clone(),
        is_partial: request.is_partial,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gent_ports::{LedgerError, TranscriptLedger};
    use gent_types::{
        AgentChatConversationId, AgentChatRunId, NormalizedTranscriptAppend,
        NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    };

    use super::{
        AgentChatTranscriptAppendRequest, AgentChatTranscriptAppendResult,
        AgentChatTranscriptAuthority, AgentChatTranscriptIngress,
    };

    #[derive(Clone, Default)]
    struct Ledger(Arc<Mutex<Vec<(AgentChatConversationId, NormalizedTranscriptAppend)>>>);

    impl TranscriptLedger for Ledger {
        fn append_normalized_transcript(
            &self,
            conversation_id: &AgentChatConversationId,
            append: &NormalizedTranscriptAppend,
        ) -> Result<NormalizedTranscriptEvent, LedgerError> {
            self.0
                .lock()
                .expect("test ledger mutex")
                .push((conversation_id.clone(), append.clone()));
            Ok(NormalizedTranscriptEvent {
                cursor: 7,
                event_id: append.event_id.clone(),
                turn_id: append.turn_id.clone(),
                run_id: append.run_id.clone(),
                kind: append.kind,
                text: append.text.clone(),
                is_partial: append.is_partial,
            })
        }

        fn normalized_transcript_page(
            &self,
            _: &AgentChatConversationId,
            _: u64,
            _: u16,
        ) -> Result<NormalizedTranscriptPage, LedgerError> {
            unreachable!("append-only test ledger")
        }
    }

    #[test]
    fn observer_denies_before_storage() {
        let ledger = Ledger::default();
        let result =
            AgentChatTranscriptIngress::new(ledger.clone(), AgentChatTranscriptAuthority::Observer)
                .append(&request())
                .unwrap();
        assert_eq!(result, AgentChatTranscriptAppendResult::DeniedObserver);
        assert!(ledger.0.lock().expect("test ledger mutex").is_empty());
    }

    #[test]
    fn approved_ingress_binds_content_to_the_requested_hierarchy() {
        let ledger = Ledger::default();
        let result =
            AgentChatTranscriptIngress::new(ledger.clone(), AgentChatTranscriptAuthority::Approved)
                .append(&request())
                .unwrap();
        assert!(matches!(
            result,
            AgentChatTranscriptAppendResult::Persisted(NormalizedTranscriptEvent { cursor: 7, .. })
        ));
        assert_eq!(
            ledger.0.lock().expect("test ledger mutex").as_slice(),
            &[(
                AgentChatConversationId("conversation-1".into()),
                NormalizedTranscriptAppend {
                    event_id: "event-1".into(),
                    turn_id: "turn-1".into(),
                    run_id: "run-1".into(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: "done".into(),
                    is_partial: false,
                },
            )]
        );
    }

    fn request() -> AgentChatTranscriptAppendRequest {
        AgentChatTranscriptAppendRequest {
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            event_id: "event-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "done".into(),
            is_partial: false,
        }
    }
}
