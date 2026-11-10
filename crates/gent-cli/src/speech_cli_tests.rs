use gent_protocol::{
    read_json_frame,
    speech_catalog::{SPEECH_CATALOG_CAPABILITY, SpeechCatalog, SpeechCatalogFrame},
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::{Value, json};
use tokio::io::duplex;

use super::read_catalog;
use crate::cli_error::CliError;

fn shipped() -> SpeechCatalog {
    SpeechCatalog::from_json(include_str!("../../gentd/speech-catalog.json")).unwrap()
}

fn negotiated() -> CapabilitySet {
    CapabilitySet(vec![SPEECH_CATALOG_CAPABILITY.into()])
}

async fn exchange(
    capabilities: CapabilitySet,
    reply: impl FnOnce(String) -> Value + Send + 'static,
) -> Result<Value, String> {
    let (client, mut daemon) = duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(SpeechCatalogFrame::ReadSpeechCatalog { request_id }) =
            read_json_frame(&mut daemon).await
        {
            write_json_frame(&mut daemon, &reply(request_id))
                .await
                .unwrap();
        }
    });
    read_catalog(client, &capabilities).await.map_err(|error| {
        match error.downcast_ref::<CliError>() {
            Some(error) => error.code().unwrap_or_default().to_owned(),
            None => error.to_string(),
        }
    })
}

#[tokio::test]
async fn the_printed_catalogue_deserializes_to_the_catalogue_gentd_served() {
    let served = shipped();
    let revision = served.revision();
    let catalog = served.clone();
    let printed = exchange(negotiated(), move |request_id| {
        serde_json::to_value(SpeechCatalogFrame::SpeechCatalog {
            request_id,
            revision,
            catalog,
        })
        .unwrap()
    })
    .await
    .unwrap();
    let text = serde_json::to_string_pretty(&printed).unwrap();
    let reparsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reparsed["revision"], served.revision());
    let catalog: SpeechCatalog = serde_json::from_value(reparsed["catalog"].clone()).unwrap();
    assert_eq!(catalog, served);
}

#[tokio::test]
async fn a_catalogue_whose_revision_does_not_match_is_rejected() {
    let catalog = shipped();
    let error = exchange(negotiated(), move |request_id| {
        serde_json::to_value(SpeechCatalogFrame::SpeechCatalog {
            request_id,
            revision: "0".repeat(64),
            catalog,
        })
        .unwrap()
    })
    .await
    .unwrap_err();
    assert!(error.contains("revision"), "{error}");
}

#[tokio::test]
async fn a_daemon_error_keeps_its_code() {
    let error = exchange(negotiated(), |_| {
        json!({"type": "error", "body": {"code": "speechCatalogUnavailable", "message": "no catalogue"}})
    })
    .await
    .unwrap_err();
    assert_eq!(error, "speechCatalogUnavailable");
}

#[test]
fn speech_catalog_is_a_public_command() {
    use clap::Parser;
    let args = crate::Args::try_parse_from(["gent", "speech", "catalog"]).unwrap();
    assert!(matches!(
        args.command,
        Some(crate::CommandLine::Speech {
            action: super::SpeechCommand::Catalog
        })
    ));
}

#[tokio::test]
async fn a_daemon_without_the_capability_is_not_asked() {
    let error = exchange(CapabilitySet(vec!["events".into()]), |_| Value::Null)
        .await
        .unwrap_err();
    assert_eq!(error, "gentd does not expose the speech catalogue");
}
