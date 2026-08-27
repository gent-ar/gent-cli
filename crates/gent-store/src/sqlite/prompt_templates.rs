use gent_ports::{LedgerError, PromptTemplateLedger};
use gent_types::{PROMPT_TEMPLATE_SCHEMA_VERSION, PromptTemplateRecord};
use rusqlite::{OptionalExtension, params};

use super::{SqliteLedger, queries::storage_error};

impl PromptTemplateLedger for SqliteLedger {
    fn create_prompt_template(
        &self,
        template: &PromptTemplateRecord,
    ) -> Result<PromptTemplateRecord, LedgerError> {
        template
            .validate()
            .map_err(|_| LedgerError::Invariant("prompt template metadata".into()))?;
        let connection = self.lock()?;
        connection.execute("INSERT INTO prompt_templates (template_id, schema_version, name, body) VALUES (?1, ?2, ?3, ?4)", params![template.template_id, template.schema_version, template.name, template.body]).map_err(storage_error)?;
        Ok(template.clone())
    }

    fn list_prompt_templates(&self) -> Result<Vec<PromptTemplateRecord>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT template_id, schema_version, name, body FROM prompt_templates ORDER BY creation_order").map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(PromptTemplateRecord {
                    schema_version: row.get(1)?,
                    template_id: row.get(0)?,
                    name: row.get(2)?,
                    body: row.get(3)?,
                })
            })
            .map_err(storage_error)?;
        rows.map(|row| row.map_err(storage_error).and_then(validate))
            .collect()
    }

    fn find_prompt_template(
        &self,
        template_id: &str,
    ) -> Result<Option<PromptTemplateRecord>, LedgerError> {
        let connection = self.lock()?;
        connection.query_row("SELECT template_id, schema_version, name, body FROM prompt_templates WHERE template_id = ?1", [template_id], |row| Ok(PromptTemplateRecord { schema_version: row.get(1)?, template_id: row.get(0)?, name: row.get(2)?, body: row.get(3)? })).optional().map_err(storage_error)?.map(validate).transpose()
    }

    fn delete_prompt_template(&self, template_id: &str) -> Result<bool, LedgerError> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM prompt_templates WHERE template_id = ?1",
                [template_id],
            )
            .map(|count| count == 1)
            .map_err(storage_error)
    }
}

fn validate(template: PromptTemplateRecord) -> Result<PromptTemplateRecord, LedgerError> {
    (template.schema_version == PROMPT_TEMPLATE_SCHEMA_VERSION)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("stored prompt template schema".into()))?;
    template
        .validate()
        .map_err(|_| LedgerError::Invariant("stored prompt template metadata".into()))?;
    Ok(template)
}
