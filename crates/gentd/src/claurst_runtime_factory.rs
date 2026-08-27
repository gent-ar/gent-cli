use async_trait::async_trait;
use gent_types::{AgentChatPromptSaved, AttachmentMetadata};

#[async_trait]
pub(crate) trait ClaurstRuntimeFactory: Send + Sync + std::fmt::Debug {
    async fn ensure_for_prompt(&self, saved: &AgentChatPromptSaved) -> Result<(), String>;
    async fn after_prompt_settled(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn after_prompt_failed(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn prompt_attachments(
        &self,
        metadata: &[AttachmentMetadata],
    ) -> Result<Vec<gent_ports::ClaurstPromptAttachment>, String> {
        if metadata.is_empty() {
            Ok(Vec::new())
        } else {
            Err("selected Claurst runtime cannot project attachments".into())
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ReadyClaurstRuntime;

#[async_trait]
impl ClaurstRuntimeFactory for ReadyClaurstRuntime {
    async fn ensure_for_prompt(&self, _: &AgentChatPromptSaved) -> Result<(), String> {
        Ok(())
    }
}
