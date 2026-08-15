//! Git adapter boundary. Mutating operations are intentionally not implemented in this milestone.

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git support is not enabled")]
    Disabled,
}
