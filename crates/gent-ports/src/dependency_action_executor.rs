//! Narrow boundary for a daemon-owned, explicitly consented dependency effect.

/// Fixed public-provider operation selected after plan verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyActionOperation {
    pub provider: String,
    pub action: String,
}

/// A known failure from a vendor dependency operation.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("dependency action failed: {message}")]
pub struct DependencyActionExecutorError {
    pub message: String,
}

/// Executes a prevalidated, daemon-owned public provider operation.
pub trait DependencyActionExecutor: Send + Sync {
    /// Runs the fixed operation without accepting shell text or client executable paths.
    ///
    /// # Errors
    /// Returns an error when the fixed vendor command fails.
    fn execute(
        &self,
        operation: &DependencyActionOperation,
    ) -> Result<(), DependencyActionExecutorError>;
}
