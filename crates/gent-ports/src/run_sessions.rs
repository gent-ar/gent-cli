//! Durable provider-native session identities owned by a run, never by a client request.

/// Immutable provider session identity attached by the daemon after a provider reports it.
///
/// A provider switch creates a child run, so this binding is never rewritten for another
/// provider or session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSessionBinding {
    pub run_id: String,
    pub provider_session_id: String,
}
