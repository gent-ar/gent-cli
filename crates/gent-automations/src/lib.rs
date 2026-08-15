//! Pure automation policy. This crate never starts schedules, scripts, or listeners.

mod policy;

pub use policy::*;

#[cfg(test)]
mod tests;
