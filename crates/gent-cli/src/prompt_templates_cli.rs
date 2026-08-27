use std::path::PathBuf;

use clap::Subcommand;
use gent_protocol::{PROMPT_TEMPLATES_CAPABILITY, PromptTemplateFrame};
use gent_types::{
    PROMPT_TEMPLATE_SCHEMA_VERSION, PromptTemplateRecord, PromptTemplateRender,
    PromptTemplateVariable,
};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

#[derive(Debug, Subcommand)]
pub(crate) enum PromptTemplateCommand {
    Create {
        template_id: String,
        name: String,
        body: String,
    },
    List,
    Get {
        template_id: String,
    },
    Delete {
        template_id: String,
    },
    Render {
        template_id: String,
        #[arg(long = "var", value_parser = parse_variable)]
        variables: Vec<PromptTemplateVariable>,
    },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: PromptTemplateCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let frame = match command {
        PromptTemplateCommand::Create {
            template_id,
            name,
            body,
        } => PromptTemplateFrame::Create {
            request_id,
            template: PromptTemplateRecord {
                schema_version: PROMPT_TEMPLATE_SCHEMA_VERSION,
                template_id,
                name,
                body,
            },
        },
        PromptTemplateCommand::List => PromptTemplateFrame::List { request_id },
        PromptTemplateCommand::Get { template_id } => PromptTemplateFrame::Get {
            request_id,
            template_id,
        },
        PromptTemplateCommand::Delete { template_id } => PromptTemplateFrame::Delete {
            request_id,
            template_id,
        },
        PromptTemplateCommand::Render {
            template_id,
            variables,
        } => PromptTemplateFrame::Render {
            request_id,
            render: PromptTemplateRender {
                template_id,
                variables,
            },
        },
    };
    frame.validate()?;
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == PROMPT_TEMPLATES_CAPABILITY)
    {
        return Err("gentd does not expose prompt templates".into());
    }
    gent_protocol::write_json_frame(&mut stream, &frame).await?;
    let raw: Value = gent_protocol::read_json_frame(&mut stream).await?;
    if let Ok(reply) = serde_json::from_value::<PromptTemplateFrame>(raw.clone()) {
        return Ok(serde_json::to_value(reply)?);
    }
    if let Ok(gent_protocol::WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a prompt template response".into())
}

pub(crate) async fn render(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    template_id: String,
    variables: Vec<PromptTemplateVariable>,
) -> Result<String, Box<dyn std::error::Error>> {
    let value = execute(
        data_dir,
        no_autostart,
        PromptTemplateCommand::Render {
            template_id,
            variables,
        },
    )
    .await?;
    let frame: PromptTemplateFrame = serde_json::from_value(value)?;
    match frame {
        PromptTemplateFrame::Rendered { prompt, .. } => Ok(prompt),
        _ => Err("daemon returned an unexpected template response".into()),
    }
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<Vec<PromptTemplateRecord>, Box<dyn std::error::Error>> {
    let value = execute(data_dir, no_autostart, PromptTemplateCommand::List).await?;
    let frame: PromptTemplateFrame = serde_json::from_value(value)?;
    match frame {
        PromptTemplateFrame::Templates { templates, .. } => Ok(templates),
        _ => Err("invalid prompt template response".into()),
    }
}

fn parse_variable(value: &str) -> Result<PromptTemplateVariable, String> {
    let Some((name, value)) = value.split_once('=') else {
        return Err("template variables must use name=value".into());
    };
    Ok(PromptTemplateVariable {
        name: name.into(),
        value: value.into(),
    })
}
