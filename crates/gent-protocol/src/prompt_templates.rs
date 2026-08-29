use gent_types::{PromptTemplateRecord, PromptTemplateRender};
use serde::{Deserialize, Serialize};

pub const PROMPT_TEMPLATES_CAPABILITY: &str = "prompt-templates-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum PromptTemplateFrame {
    Create {
        request_id: String,
        template: PromptTemplateRecord,
    },
    Created {
        request_id: String,
        template: PromptTemplateRecord,
    },
    List {
        request_id: String,
    },
    Templates {
        request_id: String,
        templates: Vec<PromptTemplateRecord>,
    },
    Get {
        request_id: String,
        template_id: String,
    },
    Template {
        request_id: String,
        template: Option<PromptTemplateRecord>,
    },
    Delete {
        request_id: String,
        template_id: String,
    },
    Deleted {
        request_id: String,
        template_id: String,
    },
    Render {
        request_id: String,
        render: PromptTemplateRender,
    },
    Rendered {
        request_id: String,
        template_id: String,
        prompt: String,
    },
}

impl PromptTemplateFrame {
    /// # Errors
    ///
    /// Returns an error when the frame's request or template data is invalid.
    pub fn validate(&self) -> Result<(), &'static str> {
        let request_id = match self {
            Self::Create { request_id, .. }
            | Self::Created { request_id, .. }
            | Self::List { request_id }
            | Self::Templates { request_id, .. }
            | Self::Get { request_id, .. }
            | Self::Template { request_id, .. }
            | Self::Delete { request_id, .. }
            | Self::Deleted { request_id, .. }
            | Self::Render { request_id, .. }
            | Self::Rendered { request_id, .. } => request_id,
        };
        if request_id.is_empty()
            || request_id.len() > 128
            || request_id.chars().any(char::is_control)
        {
            return Err("prompt template request identifier is invalid");
        }
        match self {
            Self::Create { template, .. } | Self::Created { template, .. } => {
                template
                    .validate()
                    .map_err(|_| "prompt template is invalid")?;
            }
            Self::Render { render, .. } => {
                if render.template_id.is_empty() || render.variables.len() > 32 {
                    return Err("prompt template render is invalid");
                }
                for variable in &render.variables {
                    if variable.name.is_empty()
                        || variable.name.len() > 64
                        || variable.value.len() > 16 * 1024
                    {
                        return Err("prompt template variable is invalid");
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
