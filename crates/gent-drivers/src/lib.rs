//! Public Claude and Codex driver contracts will live here; no driver is enabled yet.

pub mod lock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicProvider {
    Claude,
    Codex,
}
