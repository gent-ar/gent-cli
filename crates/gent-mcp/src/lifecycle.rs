//! Pure MCP lifecycle reduction. Effects are instructions, never side effects.

/// Authority level selected by the composition root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpMode {
    /// Observer hosts cannot connect to, spawn, or otherwise activate MCP connectors.
    Observer,
    /// A later authority-gated host may execute explicitly returned effects.
    Authority,
}

/// Durable-in-memory lifecycle projection for one MCP hosting domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpState {
    /// The only state permitted while the daemon is an observer.
    HardDisabled,
    Stopped,
    Connecting,
    Ready,
    Failed,
}

/// Facts supplied by a transport owner to the lifecycle reducer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpEvent {
    StartRequested,
    Connected,
    ConnectionFailed,
    StopRequested,
    Stopped,
}

/// One permitted instruction for an authority-mode transport owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpEffect {
    None,
    EstablishConnections,
    CloseConnections,
}

/// Complete result of reducing an MCP lifecycle event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpTransition {
    pub state: McpState,
    pub effect: McpEffect,
    pub accepted: bool,
}

/// Returns the only legal initial lifecycle state for a mode.
#[must_use]
pub const fn initial_state(mode: McpMode) -> McpState {
    match mode {
        McpMode::Observer => McpState::HardDisabled,
        McpMode::Authority => McpState::Stopped,
    }
}

/// Reduces one lifecycle fact without I/O.
///
/// Observer mode always returns [`McpState::HardDisabled`] and
/// [`McpEffect::None`]. Therefore an observer-mode caller cannot receive an
/// instruction that could spawn a stdio connector or open a network connection.
#[must_use]
pub const fn transition(mode: McpMode, state: McpState, event: McpEvent) -> McpTransition {
    if matches!(mode, McpMode::Observer) {
        return McpTransition {
            state: McpState::HardDisabled,
            effect: McpEffect::None,
            accepted: false,
        };
    }

    match (state, event) {
        (McpState::Stopped | McpState::Failed, McpEvent::StartRequested) => McpTransition {
            state: McpState::Connecting,
            effect: McpEffect::EstablishConnections,
            accepted: true,
        },
        (McpState::Connecting, McpEvent::Connected) => McpTransition {
            state: McpState::Ready,
            effect: McpEffect::None,
            accepted: true,
        },
        (McpState::Connecting | McpState::Ready, McpEvent::ConnectionFailed) => McpTransition {
            state: McpState::Failed,
            effect: McpEffect::None,
            accepted: true,
        },
        (McpState::Connecting | McpState::Ready | McpState::Failed, McpEvent::StopRequested) => {
            McpTransition {
                state: McpState::Stopped,
                effect: McpEffect::CloseConnections,
                accepted: true,
            }
        }
        (McpState::Stopped, McpEvent::Stopped) => McpTransition {
            state,
            effect: McpEffect::None,
            accepted: true,
        },
        _ => McpTransition {
            state,
            effect: McpEffect::None,
            accepted: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{McpEffect, McpEvent, McpMode, McpState, initial_state, transition};

    #[test]
    fn observer_mode_hard_disables_every_lifecycle_event() {
        for event in [
            McpEvent::StartRequested,
            McpEvent::Connected,
            McpEvent::ConnectionFailed,
            McpEvent::StopRequested,
            McpEvent::Stopped,
        ] {
            let result = transition(McpMode::Observer, McpState::Ready, event);
            assert_eq!(result.state, McpState::HardDisabled);
            assert_eq!(result.effect, McpEffect::None);
            assert!(!result.accepted);
        }
    }

    #[test]
    fn authority_mode_emits_connection_instruction_only_after_start() {
        assert_eq!(initial_state(McpMode::Authority), McpState::Stopped);
        assert_eq!(
            transition(
                McpMode::Authority,
                McpState::Stopped,
                McpEvent::StartRequested
            ),
            super::McpTransition {
                state: McpState::Connecting,
                effect: McpEffect::EstablishConnections,
                accepted: true,
            }
        );
    }

    #[test]
    fn invalid_events_do_not_emit_effects() {
        let result = transition(McpMode::Authority, McpState::Stopped, McpEvent::Connected);
        assert!(!result.accepted);
        assert_eq!(result.effect, McpEffect::None);
    }
}
