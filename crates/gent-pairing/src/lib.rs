//! Pure pairing transport policy.
//!
//! This crate deliberately owns neither sockets nor device discovery. A transport
//! adapter uses these values to apply the same protocol contract as local IPC.

mod handshake;
mod replay;

pub use handshake::{PairingHandshake, PairingHandshakeError};
pub use replay::{EventAcknowledgement, PairingSession, ReplayError, SessionConnection};

/// Pairing is an explicit transport boundary, not a second runtime authority.
pub const PAIRING_ENABLED: bool = false;
