use gent_drivers::public_protocol::PublicWireFact;
use gent_types::{DurableTurnPhase, NormalizedLifecycleSignal, TurnPhase};

pub(super) fn terminal(fact: &PublicWireFact) -> Option<DurableTurnPhase> {
    match fact {
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready,
        }) => Some(DurableTurnPhase::Completed),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Interrupted,
        }) => Some(DurableTurnPhase::Interrupted),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Failed,
        }) => Some(DurableTurnPhase::Failed),
        _ => None,
    }
}
