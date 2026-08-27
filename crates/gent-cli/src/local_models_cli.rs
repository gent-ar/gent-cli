use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
};

use clap::Subcommand;
use gent_protocol::{
    LOCAL_MODELS_CAPABILITY, LocalModelDescriptor, LocalModelFrame, LocalModelInstallState,
    WireFrame, read_json_frame, write_json_frame,
};
use serde_json::Value;

use crate::local_ipc::{LocalStream, connect_and_negotiate};

#[derive(Debug, Subcommand)]
pub(crate) enum LocalModelsCommand {
    List,
    Status { model_id: String },
    Download { model_id: String },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: LocalModelsCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    match command {
        LocalModelsCommand::List => Ok(serde_json::to_value(list(data_dir, no_autostart).await?)?),
        LocalModelsCommand::Status { model_id } => Ok(serde_json::to_value(
            status(data_dir, no_autostart, model_id).await?,
        )?),
        LocalModelsCommand::Download { model_id } => Ok(serde_json::to_value(
            download(data_dir, no_autostart, model_id).await?,
        )?),
    }
}

pub(crate) async fn download(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
) -> Result<Vec<LocalModelFrame>, Box<dyn std::error::Error>> {
    let mut frames = Vec::new();
    download_with_report(data_dir, no_autostart, model_id, |frame| {
        frames.push(frame.clone());
    })
    .await?;
    Ok(frames)
}

pub(crate) async fn download_with_report(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
    mut report: impl FnMut(&LocalModelFrame),
) -> Result<(), Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let (mut stream, _) = connect_local_models(data_dir, no_autostart).await?;
    write_json_frame(
        &mut stream,
        &LocalModelFrame::Download {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
        },
    )
    .await?;
    let mut accepted = false;
    loop {
        let reply = read_local_model_reply(&mut stream).await?;
        let correlated = match &reply {
            LocalModelFrame::DownloadAccepted {
                request_id: id,
                model_id: model,
                ..
            }
            | LocalModelFrame::DownloadProgress {
                request_id: id,
                model_id: model,
                ..
            }
            | LocalModelFrame::DownloadComplete {
                request_id: id,
                model_id: model,
                ..
            }
            | LocalModelFrame::DownloadFailed {
                request_id: id,
                model_id: model,
                ..
            } => id == &request_id && model == &model_id,
            _ => false,
        };
        if !correlated {
            return Err("daemon did not return a correlated local-model download frame".into());
        }
        match reply {
            LocalModelFrame::DownloadAccepted { .. } if !accepted => accepted = true,
            LocalModelFrame::DownloadProgress { .. } if accepted => {}
            LocalModelFrame::DownloadComplete { .. } | LocalModelFrame::DownloadFailed { .. }
                if accepted =>
            {
                report(&reply);
                return Ok(());
            }
            _ => return Err("daemon returned an invalid local-model download sequence".into()),
        }
        report(&reply);
    }
}

pub(crate) async fn download_to_stdout(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut failure = None;
    let interactive = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    download_with_report(data_dir, no_autostart, model_id, |frame| {
        render_download_progress(frame);
        if !interactive {
            let _ = crate::command_execution::print(frame);
        }
        if let LocalModelFrame::DownloadFailed { reason, .. } = frame {
            failure = Some(*reason);
        }
    })
    .await?;
    if let Some(reason) = failure {
        return Err(format!("local model download failed: {reason:?}").into());
    }
    Ok(())
}

fn render_download_progress(frame: &LocalModelFrame) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    let Some((line, terminal)) = download_display(frame) else {
        return;
    };
    let stderr = std::io::stderr();
    let mut output = stderr.lock();
    if terminal {
        let _ = writeln!(output, "\r{line}");
    } else {
        let _ = write!(output, "\r{line}");
        let _ = output.flush();
    }
}

fn download_display(frame: &LocalModelFrame) -> Option<(String, bool)> {
    match frame {
        LocalModelFrame::DownloadAccepted {
            model_id,
            state:
                LocalModelInstallState::Downloading {
                    downloaded_bytes,
                    total_bytes,
                },
            ..
        }
        | LocalModelFrame::DownloadProgress {
            model_id,
            downloaded_bytes,
            total_bytes,
            ..
        } => Some((
            progress_display(model_id, *downloaded_bytes, *total_bytes),
            false,
        )),
        LocalModelFrame::DownloadAccepted {
            model_id,
            state: LocalModelInstallState::Ready { size_bytes },
            ..
        }
        | LocalModelFrame::DownloadComplete {
            model_id,
            size_bytes,
            ..
        } => Some((
            format!("Downloaded {model_id} · 100% ({})", bytes(*size_bytes)),
            true,
        )),
        LocalModelFrame::DownloadFailed {
            model_id, reason, ..
        } => Some((format!("Could not download {model_id} · {reason:?}"), true)),
        _ => None,
    }
}

fn progress_display(model_id: &str, downloaded_bytes: u64, total_bytes: u64) -> String {
    if total_bytes == 0 {
        return format!("Downloading {model_id} · {}", bytes(downloaded_bytes));
    }
    let percent = (u128::from(downloaded_bytes) * 100 / u128::from(total_bytes)).min(100);
    format!(
        "Downloading {model_id} · {percent}% ({} / {})",
        bytes(downloaded_bytes),
        bytes(total_bytes)
    )
}

fn bytes(value: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if value >= GIB {
        scaled_bytes(value, GIB, "GiB")
    } else if value >= MIB {
        scaled_bytes(value, MIB, "MiB")
    } else {
        format!("{value} B")
    }
}

fn scaled_bytes(value: u64, unit: u64, label: &str) -> String {
    let mut whole = value / unit;
    let mut tenths = ((value % unit) * 10 + unit / 2) / unit;
    if tenths == 10 {
        whole += 1;
        tenths = 0;
    }
    format!("{whole}.{tenths} {label}")
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<Vec<LocalModelDescriptor>, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let reply = exchange(
        data_dir,
        no_autostart,
        LocalModelFrame::List {
            request_id: request_id.clone(),
        },
    )
    .await?;
    match reply {
        LocalModelFrame::Catalogue {
            request_id: reply_id,
            models,
        } if reply_id == request_id => Ok(models),
        _ => Err("daemon did not return a correlated local-model catalogue".into()),
    }
}

pub(crate) async fn status(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
) -> Result<LocalModelInstallState, Box<dyn std::error::Error>> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let reply = exchange(
        data_dir,
        no_autostart,
        LocalModelFrame::Status {
            request_id: request_id.clone(),
            model_id: model_id.clone(),
        },
    )
    .await?;
    match reply {
        LocalModelFrame::StatusResult {
            request_id: reply_id,
            model_id: reply_model,
            state,
        } if reply_id == request_id && reply_model == model_id => Ok(state),
        _ => Err("daemon did not return a correlated local-model status".into()),
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: LocalModelFrame,
) -> Result<LocalModelFrame, Box<dyn std::error::Error>> {
    frame.validate()?;
    let (mut stream, _) = connect_local_models(data_dir, no_autostart).await?;
    write_json_frame(&mut stream, &frame).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    decode_local_model_reply(raw)
}

async fn connect_local_models(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(LocalStream, gent_types::CapabilitySet), Box<dyn std::error::Error>> {
    let (stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == LOCAL_MODELS_CAPABILITY)
    {
        return Err("gentd does not expose curated local models for this authority profile".into());
    }
    Ok((stream, capabilities))
}

async fn read_local_model_reply(
    stream: &mut LocalStream,
) -> Result<LocalModelFrame, Box<dyn std::error::Error>> {
    decode_local_model_reply(read_json_frame::<_, Value>(stream).await?)
}

fn decode_local_model_reply(raw: Value) -> Result<LocalModelFrame, Box<dyn std::error::Error>> {
    if let Ok(reply) = serde_json::from_value::<LocalModelFrame>(raw.clone()) {
        reply.validate()?;
        return Ok(reply);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return a local-model response".into())
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{Hello, Negotiated, read_frame, write_frame};
    use gent_types::{CapabilitySet, PROTOCOL_MAX};
    use tokio::net::UnixListener;

    use super::*;

    #[test]
    fn unknown_download_size_has_a_safe_display() {
        assert_eq!(progress_display("qwen3", 12, 0), "Downloading qwen3 · 12 B");
    }

    #[tokio::test]
    async fn list_negotiates_capability_and_returns_catalogue() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(
                matches!(read_frame(&mut stream).await.unwrap(), WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&LOCAL_MODELS_CAPABILITY.into()))
            );
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![LOCAL_MODELS_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            let LocalModelFrame::List { request_id } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected list")
            };
            write_json_frame(
                &mut stream,
                &LocalModelFrame::Catalogue {
                    request_id,
                    models: vec![LocalModelDescriptor {
                        id: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
                        label: "Qwen2.5-Coder 7B Instruct Q4_K_M".into(),
                        size_bytes: 10,
                    }],
                },
            )
            .await
            .unwrap();
        });
        assert_eq!(
            list(Some(directory.path().into()), true).await.unwrap()[0].id,
            "qwen2-5-coder-7b-instruct-q4-k-m"
        );
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
}
