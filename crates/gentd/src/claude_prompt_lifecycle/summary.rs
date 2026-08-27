use gent_runtime::RuntimeError;

pub(crate) trait ClaudeSummaryHook: std::fmt::Debug + Send + Sync {
    fn schedule(&self, conversation_id: &str) -> Result<(), RuntimeError>;
}
