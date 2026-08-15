//! Capability adaptation for a paired transport.

use gent_protocol::{Hello, Negotiated, ProtocolError, negotiate};
use gent_types::CapabilitySet;

/// Applies the capability envelope available to one paired connection.
///
/// The envelope may only remove capabilities. It can never make a daemon or
/// peer support a capability that was not offered by both sides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairingHandshake {
    transport_capabilities: CapabilitySet,
}

impl PairingHandshake {
    #[must_use]
    pub const fn new(transport_capabilities: CapabilitySet) -> Self {
        Self {
            transport_capabilities,
        }
    }

    /// Removes capabilities unavailable on this paired transport from a hello.
    #[must_use]
    pub fn adapt_hello(&self, hello: &Hello) -> Hello {
        Hello {
            protocol_min: hello.protocol_min,
            protocol_max: hello.protocol_max,
            capabilities: hello
                .capabilities
                .intersection(&self.transport_capabilities),
        }
    }

    /// Negotiates protocol and capabilities without changing protocol semantics.
    ///
    /// # Errors
    /// Returns an error when the peer and daemon protocol ranges do not overlap.
    pub fn negotiate(
        &self,
        hello: &Hello,
        daemon_protocol_min: u16,
        daemon_protocol_max: u16,
        daemon_capabilities: &CapabilitySet,
    ) -> Result<Negotiated, PairingHandshakeError> {
        let paired_hello = self.adapt_hello(hello);
        let paired_daemon = daemon_capabilities.intersection(&self.transport_capabilities);
        negotiate(
            &paired_hello,
            daemon_protocol_min,
            daemon_protocol_max,
            &paired_daemon,
        )
        .map_err(PairingHandshakeError::Protocol)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PairingHandshakeError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[cfg(test)]
mod tests {
    use super::PairingHandshake;
    use gent_protocol::Hello;
    use gent_types::CapabilitySet;

    fn capabilities(names: &[&str]) -> CapabilitySet {
        CapabilitySet(names.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn transport_envelope_only_removes_capabilities() {
        let pairing = PairingHandshake::new(capabilities(&["events", "receipts"]));
        let hello = Hello {
            protocol_min: 1,
            protocol_max: 2,
            capabilities: capabilities(&["events", "admin"]),
        };

        let negotiated = pairing
            .negotiate(
                &hello,
                1,
                1,
                &capabilities(&["events", "admin", "receipts"]),
            )
            .unwrap();

        assert_eq!(negotiated.protocol, 1);
        assert_eq!(negotiated.capabilities, capabilities(&["events"]));
    }

    #[test]
    fn incompatible_protocols_are_not_hidden_by_adaptation() {
        let pairing = PairingHandshake::new(capabilities(&["events"]));
        let hello = Hello {
            protocol_min: 2,
            protocol_max: 3,
            capabilities: capabilities(&["events"]),
        };

        assert!(
            pairing
                .negotiate(&hello, 1, 1, &capabilities(&["events"]))
                .is_err()
        );
    }
}
