use gent_ports::PromptTemplateLedger;
use gent_types::{PromptTemplateRecord, PromptTemplateRender};

#[derive(Clone, Debug)]
pub struct PromptTemplateService<L> {
    ledger: L,
}

impl<L: PromptTemplateLedger> PromptTemplateService<L> {
    pub fn new(ledger: L) -> Self {
        Self { ledger }
    }
    pub fn create(&self, template: PromptTemplateRecord) -> Result<PromptTemplateRecord, String> {
        self.ledger
            .create_prompt_template(&template)
            .map_err(|error| error.to_string())
    }
    pub fn list(&self) -> Result<Vec<PromptTemplateRecord>, String> {
        self.ledger
            .list_prompt_templates()
            .map_err(|error| error.to_string())
    }
    pub fn get(&self, template_id: &str) -> Result<Option<PromptTemplateRecord>, String> {
        self.ledger
            .find_prompt_template(template_id)
            .map_err(|error| error.to_string())
    }
    pub fn delete(&self, template_id: &str) -> Result<bool, String> {
        self.ledger
            .delete_prompt_template(template_id)
            .map_err(|error| error.to_string())
    }
    pub fn render(&self, render: PromptTemplateRender) -> Result<String, String> {
        let template = self
            .get(&render.template_id)?
            .ok_or_else(|| "prompt template was not found".to_owned())?;
        template
            .render(&render.variables)
            .map_err(|error| error.to_string())
    }
}
