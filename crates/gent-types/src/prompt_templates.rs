use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROMPT_TEMPLATE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptTemplateRecord {
    pub schema_version: u16,
    pub template_id: String,
    pub name: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptTemplateVariable {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptTemplateRender {
    pub template_id: String,
    pub variables: Vec<PromptTemplateVariable>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PromptTemplateError {
    #[error("prompt template metadata is invalid or exceeds its bound")]
    InvalidMetadata,
    #[error("prompt template variable is missing: {0}")]
    MissingVariable(String),
    #[error("prompt template variable is repeated: {0}")]
    RepeatedVariable(String),
}

impl PromptTemplateRecord {
    /// # Errors
    ///
    /// Returns an error when the template's metadata or placeholders are invalid.
    pub fn validate(&self) -> Result<(), PromptTemplateError> {
        if self.schema_version != PROMPT_TEMPLATE_SCHEMA_VERSION
            || !identifier(&self.template_id)
            || !identifier(&self.name)
            || self.body.is_empty()
            || self.body.len() > 16 * 1024
            || self.body.chars().any(char::is_control)
        {
            return Err(PromptTemplateError::InvalidMetadata);
        }
        for variable in placeholders(&self.body) {
            if !identifier(&variable) {
                return Err(PromptTemplateError::InvalidMetadata);
            }
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when the template or supplied variables are invalid or incomplete.
    pub fn render(
        &self,
        variables: &[PromptTemplateVariable],
    ) -> Result<String, PromptTemplateError> {
        self.validate()?;
        if variables.len() > 32 {
            return Err(PromptTemplateError::InvalidMetadata);
        }
        let mut values = BTreeMap::new();
        for variable in variables {
            if !identifier(&variable.name)
                || variable.value.len() > 16 * 1024
                || variable.value.chars().any(char::is_control)
            {
                return Err(PromptTemplateError::InvalidMetadata);
            }
            if values
                .insert(variable.name.clone(), variable.value.clone())
                .is_some()
            {
                return Err(PromptTemplateError::RepeatedVariable(variable.name.clone()));
            }
        }
        let mut output = String::with_capacity(self.body.len());
        let mut rest = self.body.as_str();
        while let Some(start) = rest.find("{{") {
            output.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find("}}")
                .ok_or(PromptTemplateError::InvalidMetadata)?;
            let name = &after[..end];
            let value = values
                .get(name)
                .ok_or_else(|| PromptTemplateError::MissingVariable(name.into()))?;
            output.push_str(value);
            rest = &after[end + 2..];
        }
        output.push_str(rest);
        Ok(output)
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn placeholders(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return vec![String::new()];
        };
        names.push(after[..end].to_owned());
        rest = &after[end + 2..];
    }
    names
}

#[cfg(test)]
mod tests {
    use super::{
        PROMPT_TEMPLATE_SCHEMA_VERSION, PromptTemplateError, PromptTemplateRecord,
        PromptTemplateVariable,
    };

    fn template(body: &str) -> PromptTemplateRecord {
        PromptTemplateRecord {
            schema_version: PROMPT_TEMPLATE_SCHEMA_VERSION,
            template_id: "review".into(),
            name: "Review".into(),
            body: body.into(),
        }
    }

    #[test]
    fn renders_bounded_named_variables() {
        assert_eq!(
            template("Review {{file}} for {{focus}}")
                .render(&[
                    PromptTemplateVariable {
                        name: "file".into(),
                        value: "main.rs".into()
                    },
                    PromptTemplateVariable {
                        name: "focus".into(),
                        value: "safety".into()
                    }
                ])
                .unwrap(),
            "Review main.rs for safety"
        );
    }

    #[test]
    fn rejects_missing_and_repeated_variables() {
        assert_eq!(
            template("{{file}}").render(&[]),
            Err(PromptTemplateError::MissingVariable("file".into()))
        );
        assert_eq!(
            template("{{file}}").render(&[
                PromptTemplateVariable {
                    name: "file".into(),
                    value: "a".into()
                },
                PromptTemplateVariable {
                    name: "file".into(),
                    value: "b".into()
                }
            ]),
            Err(PromptTemplateError::RepeatedVariable("file".into()))
        );
    }
}
