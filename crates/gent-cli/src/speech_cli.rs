use std::path::PathBuf;

use clap::Subcommand;
use gent_protocol::{
    read_json_frame,
    speech_catalog::{SPEECH_CATALOG_CAPABILITY, SpeechCatalogFrame},
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::{Value, json};

use crate::local_ipc::connect_and_negotiate;

#[derive(Debug, Subcommand)]
pub(crate) enum SpeechCommand {
    #[command(about = "Print the speech-to-text catalogue as JSON")]
    Catalog,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: SpeechCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let SpeechCommand::Catalog = command;
    let (stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    read_catalog(stream, &capabilities).await
}

async fn read_catalog<S>(
    mut stream: S,
    capabilities: &CapabilitySet,
) -> Result<Value, Box<dyn std::error::Error>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == SPEECH_CATALOG_CAPABILITY)
    {
        return Err("gentd does not expose the speech catalogue".into());
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = SpeechCatalogFrame::ReadSpeechCatalog {
        request_id: request_id.clone(),
    };
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let reply: SpeechCatalogFrame = serde_json::from_value(raw)?;
    reply.validate()?;
    match reply {
        SpeechCatalogFrame::SpeechCatalog {
            request_id: reply_id,
            revision,
            catalog,
        } if reply_id == request_id => Ok(json!({ "revision": revision, "catalog": catalog })),
        _ => Err("daemon did not return a correlated speech catalogue".into()),
    }
}

#[cfg(test)]
#[path = "speech_cli_tests.rs"]
mod tests;
