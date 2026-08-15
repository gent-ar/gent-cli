//! Public Claude and Codex driver contracts will live here; no driver is enabled yet.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicProvider {
    Claude,
    Codex,
}
