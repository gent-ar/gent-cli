//! The ordinary owner's existing operation latch, not a separate maintenance flag.
use super::mcp::{Error, Ticket};
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;
use std::sync::Arc;

pub fn acquire(control: &OrdinaryLifecycleControl) -> Result<Ticket, Error> {
    control
        .acquire_operation()
        .map(|permit| Arc::new(permit) as Ticket)
        .map_err(|_| Error::Revoked)
}
