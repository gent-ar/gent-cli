use async_trait::async_trait;
use gent_types::Command;

use crate::PortError;

/// Legacy generic driver boundary retained for non-chat integrations.
#[async_trait]
pub trait ProviderDriver: Send + Sync {
    async fn submit(&self, command: Command) -> Result<(), PortError>;
}
