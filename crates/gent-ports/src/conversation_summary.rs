use crate::PortError;

pub trait ConversationSummaryRunner: Send + Sync {
    fn run_summary(
        &self,
        provider: &str,
        model_version: &str,
        prompt: &str,
    ) -> Result<String, PortError>;
}
