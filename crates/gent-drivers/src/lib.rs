//! Public-driver contracts and pure policies; this crate never starts a provider.

pub mod buffering;
pub mod discovery;
pub mod interrupt;
pub mod lock;
pub mod normalize;

pub use discovery::PublicProvider;
