use gent_types::PromptTemplateRecord;

use crate::LedgerError;

pub trait PromptTemplateLedger: Send + Sync {
    fn create_prompt_template(
        &self,
        template: &PromptTemplateRecord,
    ) -> Result<PromptTemplateRecord, LedgerError>;
    fn list_prompt_templates(&self) -> Result<Vec<PromptTemplateRecord>, LedgerError>;
    fn find_prompt_template(
        &self,
        template_id: &str,
    ) -> Result<Option<PromptTemplateRecord>, LedgerError>;
    fn delete_prompt_template(&self, template_id: &str) -> Result<bool, LedgerError>;
}
