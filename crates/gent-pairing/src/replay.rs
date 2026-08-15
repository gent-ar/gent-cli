//! Disconnection and cursor-replay policy for paired clients.

use gent_protocol::WireFrame;
use gent_types::{Command, Event};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionConnection {
    Disconnected,
    Connected,
}

/// Small client-side projection of the transport state, excluding transcript data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingSession {
    connection: SessionConnection,
    acknowledged_cursor: u64,
}

impl Default for PairingSession {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingSession {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            connection: SessionConnection::Disconnected,
            acknowledged_cursor: 0,
        }
    }

    #[must_use]
    pub const fn connection(&self) -> SessionConnection {
        self.connection
    }

    #[must_use]
    pub const fn acknowledged_cursor(&self) -> u64 {
        self.acknowledged_cursor
    }

    /// Prevents all command mutation while the paired transport is unavailable.
    ///
    /// # Errors
    /// Returns [`ReplayError::Disconnected`] without queuing a command offline.
    pub fn authorize_command(&self, _command: &Command) -> Result<(), ReplayError> {
        if self.connection == SessionConnection::Disconnected {
            return Err(ReplayError::Disconnected);
        }
        Ok(())
    }

    /// Marks transport unavailable. The durable cursor remains for later replay.
    pub fn disconnect(&mut self) {
        self.connection = SessionConnection::Disconnected;
    }

    /// Completes transport negotiation and requests events after the acknowledged cursor.
    #[must_use]
    pub fn reconnect(&mut self) -> WireFrame {
        self.connection = SessionConnection::Connected;
        WireFrame::Subscribe {
            after_cursor: self.acknowledged_cursor,
        }
    }

    /// Acknowledges one applied, ordered event.
    ///
    /// A duplicate is harmless, while a gap remains unacknowledged so reconnect
    /// asks the host to replay it instead of silently losing history.
    ///
    /// # Errors
    /// Returns an error for a cursor gap or when no later cursor can exist.
    pub fn acknowledge(&mut self, event: &Event) -> Result<EventAcknowledgement, ReplayError> {
        if event.cursor <= self.acknowledged_cursor {
            return Ok(EventAcknowledgement::Duplicate);
        }
        let expected = self
            .acknowledged_cursor
            .checked_add(1)
            .ok_or(ReplayError::CursorOverflow)?;
        if event.cursor != expected {
            return Err(ReplayError::CursorGap {
                expected,
                received: event.cursor,
            });
        }
        self.acknowledged_cursor = event.cursor;
        Ok(EventAcknowledgement::Advanced(event.cursor))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventAcknowledgement {
    Advanced(u64),
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReplayError {
    #[error("paired client is disconnected and cannot submit commands")]
    Disconnected,
    #[error("event cursor gap: expected {expected}, received {received}")]
    CursorGap { expected: u64, received: u64 },
    #[error("event cursor cannot advance past u64::MAX")]
    CursorOverflow,
}

#[cfg(test)]
mod tests {
    use super::{EventAcknowledgement, PairingSession, ReplayError, SessionConnection};
    use gent_protocol::WireFrame;
    use gent_types::{Command, Event, HostEpoch, ReceiptId};
    use serde_json::Value;

    fn command() -> Command {
        Command {
            receipt_id: ReceiptId("receipt".into()),
            idempotency_key: "key".into(),
            host_epoch: HostEpoch(1),
            kind: "submit".into(),
            payload: Value::Null,
        }
    }

    fn event(cursor: u64) -> Event {
        Event {
            cursor,
            event_id: format!("event-{cursor}"),
            receipt_id: ReceiptId("receipt".into()),
            host_epoch: HostEpoch(1),
            kind: "accepted".into(),
            payload: Value::Null,
        }
    }

    #[test]
    fn disconnected_clients_are_read_only_without_an_offline_queue() {
        let session = PairingSession::new();
        assert_eq!(session.connection(), SessionConnection::Disconnected);
        assert_eq!(
            session.authorize_command(&command()),
            Err(ReplayError::Disconnected)
        );
    }

    #[test]
    fn reconnect_replays_strictly_after_last_acknowledged_cursor() {
        let mut session = PairingSession::new();
        assert_eq!(
            session.reconnect(),
            WireFrame::Subscribe { after_cursor: 0 }
        );
        assert_eq!(session.authorize_command(&command()), Ok(()));
        assert_eq!(
            session.acknowledge(&event(1)),
            Ok(EventAcknowledgement::Advanced(1))
        );
        session.disconnect();
        assert_eq!(
            session.authorize_command(&command()),
            Err(ReplayError::Disconnected)
        );
        assert_eq!(
            session.reconnect(),
            WireFrame::Subscribe { after_cursor: 1 }
        );
    }

    #[test]
    fn duplicate_events_do_not_move_cursor_and_gaps_are_replayed() {
        let mut session = PairingSession::new();
        let _ = session.reconnect();
        session.acknowledge(&event(1)).unwrap();
        assert_eq!(
            session.acknowledge(&event(1)),
            Ok(EventAcknowledgement::Duplicate)
        );
        assert_eq!(
            session.acknowledge(&event(3)),
            Err(ReplayError::CursorGap {
                expected: 2,
                received: 3
            })
        );
        assert_eq!(session.acknowledged_cursor(), 1);
    }
}
