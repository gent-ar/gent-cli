use std::{io::IsTerminal, path::PathBuf};

use clap::Subcommand;
use gent_protocol::{
    LOCAL_MODELS_CAPABILITY, LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState,
    read_json_frame, write_json_frame,
};
use serde_json::Value;

use crate::{
    cli_error::{CliError, Failure},
    local_ipc::{LocalStream, connect_and_negotiate},
};

#[derive(Debug, Subcommand)]
pub(crate) enum LocalModelsCommand {
    #[command(
        about = "List Gent, Claude, and Codex models with their efforts, availability, and local install state"
    )]
    List {
        #[arg(long, help = "Print Gentd's full model catalog as JSON")]
        json: bool,
    },
    #[command(about = "Show whether a local model is installed or downloading")]
    Status {
        #[arg(help = "Local model id from `gent models list`")]
        model_id: String,
    },
    #[command(about = "Download a local model and follow its progress; Ctrl+C cancels it")]
    Download {
        #[arg(help = "Local model id from `gent models list`")]
        model_id: String,
    },
    #[command(about = "Cancel a local model download and remove its partial file")]
    Cancel {
        #[arg(help = "Local model id from `gent models list`")]
        model_id: String,
    },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: LocalModelsCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    match command {
        LocalModelsCommand::List { .. } => {
            Ok(serde_json::to_value(list(data_dir, no_autostart).await?)?)
        }
        LocalModelsCommand::Status { model_id } => Ok(serde_json::to_value(
            status(data_dir, no_autostart, model_id).await?,
        )?),
        LocalModelsCommand::Download { model_id } => Ok(serde_json::to_value(
            download(data_dir, no_autostart, model_id).await?,
        )?),
        LocalModelsCommand::Cancel { model_id } => Ok(serde_json::to_value(
            cancel(data_dir, no_autostart, model_id).await?,
        )?),
    }
}

pub(crate) async fn cancel(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    model_id: String,
) -> Result<LocalModelInstallState, Box<dyn std::error::Error>> {
    let catalog =
        crate::model_catalog_cli::cancel_download(data_dir.clone(), no_autostart, model_id.clone())
            .await?;
    if crate::model_catalog_cli::catalog_model(
        &catalog,
        gent_types::AgentChatProvider::Claurst,
        &model_id,
    )
    .is_none()
    {
        return Err(format!("{model_id} is not a Gent local model").into());
    }
    status(data_dir, no_autostart, model_id).await
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
            LocalModelFrame::DownloadFailed { .. } => {
                report(&reply);
                return Ok(());
            }
            LocalModelFrame::DownloadComplete { .. } if accepted => {
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
    let follow = download_with_report(data_dir.clone(), no_autostart, model_id.clone(), |frame| {
        render_download_progress(frame);
        if !interactive {
            let _ = crate::command_execution::print(frame);
        }
        if let LocalModelFrame::DownloadFailed { reason, .. } = frame {
            failure = Some(*reason);
        }
    });
    tokio::select! {
        followed = follow => followed?,
        interrupted = tokio::signal::ctrl_c() => {
            interrupted?;
            cancel(data_dir, no_autostart, model_id.clone()).await?;
            eprintln!();
            return Err(CliError::new(
                Failure::TurnInterrupted,
                format!("canceled the download of {model_id}"),
            )
            .into());
        }
    }
    match failure {
        None => Ok(()),
        Some(LocalModelDownloadFailure::Cancelled) => Err(CliError::new(
            Failure::TurnInterrupted,
            format!("the download of {model_id} was canceled"),
        )
        .into()),
        Some(reason) => Err(CliError::new(
            Failure::Rejected,
            format!("could not download {model_id}: {}", failure_label(reason)),
        )
        .into()),
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
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    Err("daemon did not return a local-model response".into())
}

#[path = "local_models_cli_render.rs"]
mod render;

#[path = "local_models_cli_catalog.rs"]
mod catalog;

pub(crate) use catalog::{list, render_catalog};
pub(crate) use render::progress_display;
use render::{failure_label, render_download_progress};

#[cfg(all(test, unix))]
#[path = "local_models_cli_tests.rs"]
mod tests;
