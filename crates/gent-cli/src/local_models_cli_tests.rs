use gent_protocol::{Negotiated, WireFrame, read_frame, write_frame};
use gent_types::{CapabilitySet, PROTOCOL_MAX};
use tokio::net::UnixListener;

use super::{render::download_display, *};

#[test]
fn unknown_download_size_has_a_safe_display() {
    assert_eq!(progress_display("qwen3", 12, 0), "Downloading qwen3 · 12 B");
}

#[tokio::test]
async fn status_requires_capability_before_sending_request() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet::default(),
            }),
        )
        .await
        .unwrap();
    });
    assert!(
        status(
            Some(directory.path().into()),
            true,
            "qwen2-5-coder-7b-instruct-q4-k-m".into()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("does not expose")
    );
}

#[tokio::test]
async fn download_waits_for_terminal_progress_without_exposing_private_source_data() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let LocalModelFrame::Download {
            request_id,
            model_id,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected download")
        };
        write_json_frame(
            &mut stream,
            &LocalModelFrame::DownloadAccepted {
                request_id: request_id.clone(),
                model_id: model_id.clone(),
                state: LocalModelInstallState::Downloading {
                    downloaded_bytes: 0,
                    total_bytes: 10,
                },
            },
        )
        .await
        .unwrap();
        write_json_frame(
            &mut stream,
            &LocalModelFrame::DownloadProgress {
                request_id: request_id.clone(),
                model_id: model_id.clone(),
                downloaded_bytes: 5,
                total_bytes: 10,
            },
        )
        .await
        .unwrap();
        write_json_frame(
            &mut stream,
            &LocalModelFrame::DownloadComplete {
                request_id,
                model_id,
                size_bytes: 10,
            },
        )
        .await
        .unwrap();
    });
    let frames = download(
        Some(directory.path().into()),
        true,
        "qwen2-5-coder-7b-instruct-q4-k-m".into(),
    )
    .await
    .unwrap();
    assert!(matches!(
        frames.as_slice(),
        [
            LocalModelFrame::DownloadAccepted { .. },
            LocalModelFrame::DownloadProgress {
                downloaded_bytes: 5,
                total_bytes: 10,
                ..
            },
            LocalModelFrame::DownloadComplete { size_bytes: 10, .. }
        ]
    ));
    let output = serde_json::to_string(&frames).unwrap();
    assert!(!output.contains("http"));
    assert!(!output.contains(".gguf"));
}

#[tokio::test]
async fn download_returns_typed_terminal_failure() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let LocalModelFrame::Download {
            request_id,
            model_id,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected download")
        };
        write_json_frame(
            &mut stream,
            &LocalModelFrame::DownloadAccepted {
                request_id: request_id.clone(),
                model_id: model_id.clone(),
                state: LocalModelInstallState::Downloading {
                    downloaded_bytes: 0,
                    total_bytes: 10,
                },
            },
        )
        .await
        .unwrap();
        write_json_frame(
            &mut stream,
            &LocalModelFrame::DownloadFailed {
                request_id,
                model_id,
                reason: gent_protocol::LocalModelDownloadFailure::TransportFailed,
            },
        )
        .await
        .unwrap();
    });
    let frames = download(
        Some(directory.path().into()),
        true,
        "qwen2-5-coder-7b-instruct-q4-k-m".into(),
    )
    .await
    .unwrap();
    assert!(matches!(
        frames.last(),
        Some(LocalModelFrame::DownloadFailed {
            reason: gent_protocol::LocalModelDownloadFailure::TransportFailed,
            ..
        })
    ));
}

#[test]
fn download_display_handles_resumed_large_downloads_and_completion() {
    let accepted = LocalModelFrame::DownloadAccepted {
        request_id: "download-18g".into(),
        model_id: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
        state: LocalModelInstallState::Downloading {
            downloaded_bytes: 2_341_536_832,
            total_bytes: 4_683_073_664,
        },
    };
    let (resumed, terminal) = download_display(&accepted).unwrap();
    assert_eq!(
        resumed,
        "Downloading qwen2-5-coder-7b-instruct-q4-k-m · 50% (2.2 GiB / 4.4 GiB)"
    );
    assert!(!terminal);

    let progress = LocalModelFrame::DownloadProgress {
        request_id: "download-18g".into(),
        model_id: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
        downloaded_bytes: 4_636_242_928,
        total_bytes: 4_683_073_664,
    };
    let (coalesced, terminal) = download_display(&progress).unwrap();
    assert_eq!(
        coalesced,
        "Downloading qwen2-5-coder-7b-instruct-q4-k-m · 99% (4.3 GiB / 4.4 GiB)"
    );
    assert!(!terminal);

    let complete = LocalModelFrame::DownloadComplete {
        request_id: "download-18g".into(),
        model_id: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
        size_bytes: 4_683_073_664,
    };
    let (finished, terminal) = download_display(&complete).unwrap();
    assert_eq!(
        finished,
        "Downloaded qwen2-5-coder-7b-instruct-q4-k-m · 100% (4.4 GiB)"
    );
    assert!(terminal);
}
