use std::path::PathBuf;

use gent_protocol::{
    model_catalog::{
        MODEL_CATALOG_CAPABILITY, ModelCatalog, ModelCatalogFrame, ModelCatalogSelection,
    },
    read_json_frame, write_json_frame,
};
use gent_types::AgentChatSelection;
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn read(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    read_if_exposed(data_dir, no_autostart)
        .await?
        .ok_or_else(|| CATALOG_NOT_EXPOSED.into())
}

pub(crate) async fn read_if_exposed(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<Option<ModelCatalog>, Box<dyn std::error::Error>> {
    let (stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !exposes_catalog(&capabilities) {
        return Ok(None);
    }
    exchange_on(stream, |request_id| ModelCatalogFrame::ReadModelCatalog {
        request_id,
        refresh: false,
    })
    .await
    .map(Some)
}

const CATALOG_NOT_EXPOSED: &str =
    "gentd does not expose a model catalog for this authority profile";

fn exposes_catalog(capabilities: &gent_types::CapabilitySet) -> bool {
    capabilities
        .0
        .iter()
        .any(|capability| capability == MODEL_CATALOG_CAPABILITY)
}

pub(crate) async fn set_default(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    selection: &AgentChatSelection,
) -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    let selection = ModelCatalogSelection {
        provider: selection.provider,
        model: selection.model.clone(),
    };
    exchange(data_dir, no_autostart, |request_id| {
        ModelCatalogFrame::SetDefaultModel {
            request_id,
            selection,
        }
    })
    .await
}

pub(crate) async fn cancel_download(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
) -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    exchange(data_dir, no_autostart, |request_id| {
        ModelCatalogFrame::CancelModelDownload {
            request_id,
            model_id,
        }
    })
    .await
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: impl FnOnce(String) -> ModelCatalogFrame,
) -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    let (stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !exposes_catalog(&capabilities) {
        return Err(CATALOG_NOT_EXPOSED.into());
    }
    exchange_on(stream, request).await
}

async fn exchange_on<S>(
    mut stream: S,
    request: impl FnOnce(String) -> ModelCatalogFrame,
) -> Result<ModelCatalog, Box<dyn std::error::Error>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let request_id = uuid::Uuid::new_v4().to_string();
    let frame = request(request_id.clone());
    frame.validate()?;
    write_json_frame(&mut stream, &frame).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ModelCatalogFrame::ModelCatalog {
        request_id: reply_id,
        catalog,
    }) = serde_json::from_value::<ModelCatalogFrame>(raw.clone())
        && reply_id == request_id
    {
        return Ok(catalog);
    }
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    Err("daemon did not return a correlated model catalog".into())
}

pub(crate) fn catalog_model<'a>(
    catalog: &'a ModelCatalog,
    provider: gent_types::AgentChatProvider,
    model_id: &str,
) -> Option<&'a gent_protocol::model_catalog::CatalogModel> {
    catalog
        .providers
        .iter()
        .filter(|entry| entry.provider == provider)
        .flat_map(|entry| &entry.models)
        .find(|model| model.id == model_id)
}
