use gent_protocol::{PromptTemplateFrame, WorkspaceDocumentsFrame};

use super::RuntimeFacade;

impl RuntimeFacade {
    pub(super) fn exchange_prompt_template(
        &self,
        frame: PromptTemplateFrame,
    ) -> Result<PromptTemplateFrame, String> {
        match frame {
            PromptTemplateFrame::Create {
                request_id,
                template,
            } => Ok(PromptTemplateFrame::Created {
                request_id,
                template: self.prompt_templates.create(template)?,
            }),
            PromptTemplateFrame::List { request_id } => Ok(PromptTemplateFrame::Templates {
                request_id,
                templates: self.prompt_templates.list()?,
            }),
            PromptTemplateFrame::Get {
                request_id,
                template_id,
            } => Ok(PromptTemplateFrame::Template {
                request_id,
                template: self.prompt_templates.get(&template_id)?,
            }),
            PromptTemplateFrame::Delete {
                request_id,
                template_id,
            } => {
                self.prompt_templates.delete(&template_id)?;
                Ok(PromptTemplateFrame::Deleted {
                    request_id,
                    template_id,
                })
            }
            PromptTemplateFrame::Render { request_id, render } => {
                let template_id = render.template_id.clone();
                Ok(PromptTemplateFrame::Rendered {
                    request_id,
                    template_id,
                    prompt: self.prompt_templates.render(render)?,
                })
            }
            _ => Err("prompt template response frames are server-only".into()),
        }
    }

    pub(super) fn list_workspace_documents(
        &self,
        frame: WorkspaceDocumentsFrame,
    ) -> Result<WorkspaceDocumentsFrame, String> {
        let WorkspaceDocumentsFrame::List {
            request_id,
            workspace_id,
        } = frame
        else {
            return Err("workspace document response frames are server-only".into());
        };
        let workspace = self
            .coordinator
            .workspace(&workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "workspace was not found".to_owned())?;
        let documents =
            crate::workspace_documents::discover(std::path::Path::new(&workspace.canonical_path))?;
        Ok(WorkspaceDocumentsFrame::Listed {
            request_id,
            workspace_id,
            documents,
        })
    }
}
