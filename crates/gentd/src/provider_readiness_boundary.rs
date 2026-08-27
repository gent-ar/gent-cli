use gent_protocol::ProviderReadinessFrame;

pub(crate) trait ProviderReadinessPort: Send + Sync {
    fn assess(&self, frame: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String>;
}
