use gent_protocol::{
    speech_catalog::{
        SPEECH_CATALOG_CAPABILITY, SpeechCatalog, SpeechCatalogError, SpeechCatalogFrame,
    },
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{
    api::RuntimeApi,
    transport::{off_runtime, write_error},
};

const SHIPPED_CATALOGUE: &str = include_str!("../speech-catalog.json");

pub(crate) fn shipped() -> Result<SpeechCatalog, SpeechCatalogError> {
    SpeechCatalog::from_json(SHIPPED_CATALOGUE)
}

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == SPEECH_CATALOG_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<SpeechCatalogFrame>(raw.clone()) else {
        return Ok(false);
    };
    let (Ok(()), SpeechCatalogFrame::ReadSpeechCatalog { request_id }) = (frame.validate(), frame)
    else {
        write_error(
            stream,
            "invalidSpeechCatalog",
            "speech catalog request is invalid",
        )
        .await?;
        return Ok(true);
    };
    match off_runtime(runtime, |_| shipped()).await? {
        Ok(catalog) => {
            let reply = SpeechCatalogFrame::SpeechCatalog {
                request_id,
                revision: catalog.revision(),
                catalog,
            };
            write_json_frame(stream, &reply).await?;
        }
        Err(error) => {
            write_error(stream, "speechCatalogUnavailable", &error.to_string()).await?;
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "speech_catalog_transport_tests.rs"]
mod tests;
