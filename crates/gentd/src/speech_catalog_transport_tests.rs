use gent_protocol::{
    Hello, WireFrame, read_frame, read_json_frame,
    speech_catalog::{SPEECH_CATALOG_CAPABILITY, SpeechAuthScheme, SpeechCatalogFrame},
    write_frame, write_json_frame,
};
use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
use tokio::io::{DuplexStream, duplex};

use super::shipped;
use crate::transport::{observed_capabilities, serve_connection, tests::FakeRuntime};

async fn connect(capabilities: Vec<String>) -> (DuplexStream, CapabilitySet) {
    let (mut client, server) = duplex(64 * 1024);
    tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(capabilities),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(answer) = read_frame(&mut client).await.unwrap() else {
        panic!("gentd did not negotiate");
    };
    (client, answer.capabilities)
}

fn read_request() -> SpeechCatalogFrame {
    SpeechCatalogFrame::ReadSpeechCatalog {
        request_id: "speech-1".into(),
    }
}

#[test]
fn the_shipped_catalogue_carries_every_speech_source_the_app_offered() {
    let catalog = shipped().unwrap();
    let providers: Vec<_> = catalog
        .cloud_providers
        .iter()
        .map(|provider| (provider.id.as_str(), provider.default_model_id.as_deref()))
        .collect();
    assert_eq!(
        providers,
        [
            ("openai", Some("whisper-1")),
            ("together", Some("openai/whisper-large-v3")),
            ("elevenlabs", Some("scribe_v1")),
            ("resemble", None),
            ("groq", Some("whisper-large-v3-turbo")),
            ("deepgram", Some("nova-3")),
            ("assemblyai", None),
        ]
    );
    assert_eq!(catalog.cloud_providers[3].auth, SpeechAuthScheme::Bearer);
    assert_eq!(catalog.cloud_providers[5].auth, SpeechAuthScheme::Token);
    let whisper = &catalog.local_engines[0];
    let files: Vec<_> = whisper
        .models
        .iter()
        .map(|model| (model.filename.as_str(), model.english_only))
        .collect();
    assert_eq!(
        files,
        [
            ("ggml-tiny.en.bin", true),
            ("ggml-tiny.bin", false),
            ("ggml-base.en.bin", true),
            ("ggml-base.bin", false),
            ("ggml-small.en.bin", true),
            ("ggml-small.bin", false),
        ]
    );
}

#[test]
fn every_daemon_profile_advertises_the_speech_catalogue() {
    let capabilities = observed_capabilities(&RuntimeCapabilityProfile::new([]));
    assert!(
        capabilities
            .0
            .iter()
            .any(|item| item == SPEECH_CATALOG_CAPABILITY)
    );
}

#[tokio::test]
async fn a_negotiated_client_reads_the_shipped_catalogue_with_its_revision() {
    let (mut client, negotiated) = connect(vec![SPEECH_CATALOG_CAPABILITY.into()]).await;
    assert_eq!(negotiated.0, [SPEECH_CATALOG_CAPABILITY]);
    write_json_frame(&mut client, &read_request())
        .await
        .unwrap();
    let reply: SpeechCatalogFrame = read_json_frame(&mut client).await.unwrap();
    reply.validate().unwrap();
    let SpeechCatalogFrame::SpeechCatalog {
        request_id,
        revision,
        catalog,
    } = reply
    else {
        panic!("gentd did not answer with a catalogue");
    };
    let shipped = shipped().unwrap();
    assert_eq!(request_id, "speech-1");
    assert_eq!(revision, shipped.revision());
    assert_eq!(catalog, shipped);
}

#[tokio::test]
async fn a_client_without_the_capability_is_refused() {
    let (mut client, negotiated) = connect(vec!["events".into()]).await;
    assert!(
        !negotiated
            .0
            .iter()
            .any(|item| item == SPEECH_CATALOG_CAPABILITY)
    );
    write_json_frame(&mut client, &read_request())
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidCommand"
    ));
}

#[tokio::test]
async fn clients_cannot_send_catalogue_replies_or_unbounded_request_ids() {
    let catalog = shipped().unwrap();
    for frame in [
        SpeechCatalogFrame::SpeechCatalog {
            request_id: "speech-1".into(),
            revision: catalog.revision(),
            catalog,
        },
        SpeechCatalogFrame::ReadSpeechCatalog {
            request_id: "x".repeat(129),
        },
    ] {
        let (mut client, _) = connect(vec![SPEECH_CATALOG_CAPABILITY.into()]).await;
        write_json_frame(&mut client, &frame).await.unwrap();
        assert!(matches!(
            read_frame(&mut client).await.unwrap(),
            WireFrame::Error { code, .. } if code == "invalidSpeechCatalog"
        ));
    }
}
