//! CLI composition root: maps parsed commands to the protocol-only local IPC boundary.

use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, WireFrame,
};
use gent_types::{Command, ReceiptId};
use serde_json::Value;

use crate::decision::decision_frame;
use crate::local_ipc::request;
use crate::{
    Args, CommandLine, ConversationCommand, DependencyCommand, conversation_status,
    conversation_timeline, event_stream,
};

pub(crate) async fn execute(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let Args {
        data_dir,
        no_autostart,
        command,
    } = args;
    match command {
        CommandLine::Doctor => {
            print(request(data_dir, no_autostart, WireFrame::DoctorRequest).await?)?;
        }
        CommandLine::Deps { action } => {
            print(request(data_dir, no_autostart, dependency_frame(&action)).await?)?;
        }
        CommandLine::Decision { action } => {
            print(request(data_dir, no_autostart, decision_frame(&action)).await?)?;
        }
        CommandLine::Conversation { action } => match action {
            ConversationCommand::Status { conversation_id } => {
                print(
                    conversation_status::request(data_dir, no_autostart, conversation_id).await?,
                )?;
            }
            ConversationCommand::Timeline { conversation_id } => {
                print(
                    conversation_timeline::request(data_dir, no_autostart, conversation_id).await?,
                )?;
            }
        },
        CommandLine::Status => {
            print(request(data_dir, no_autostart, WireFrame::StatusRequest).await?)?;
        }
        CommandLine::Submit {
            kind,
            payload,
            idempotency_key,
        } => submit(data_dir, no_autostart, kind, payload, idempotency_key).await?,
        CommandLine::Events {
            after_cursor,
            follow: true,
        } => event_stream::follow(data_dir, no_autostart, after_cursor).await?,
        CommandLine::Events { after_cursor, .. } => {
            print(
                request(
                    data_dir,
                    no_autostart,
                    WireFrame::Subscribe { after_cursor },
                )
                .await?,
            )?;
        }
    }
    Ok(())
}

async fn submit(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    kind: String,
    payload: String,
    idempotency_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = request(data_dir.clone(), no_autostart, WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = status else {
        return Err("daemon did not return host status".into());
    };
    let command = Command {
        receipt_id: ReceiptId::new(),
        idempotency_key: idempotency_key.unwrap_or_else(|| ReceiptId::new().0),
        host_epoch: status.host_epoch,
        kind,
        payload: serde_json::from_str::<Value>(&payload)?,
    };
    print(request(data_dir, no_autostart, WireFrame::Command(command)).await?)
}

fn print(value: impl serde::Serialize) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub(crate) fn dependency_frame(action: &DependencyCommand) -> WireFrame {
    match action {
        DependencyCommand::Plan { action, provider } => {
            WireFrame::DependencyPlanRequest(DependencyPlanRequest {
                provider: *provider,
                action: *action,
            })
        }
        DependencyCommand::Install { provider, consent } => {
            dependency_action(*provider, DependencyAction::Install, *consent)
        }
        DependencyCommand::Update { provider, consent } => {
            dependency_action(*provider, DependencyAction::Update, *consent)
        }
    }
}

fn dependency_action(
    provider: DependencyProvider,
    action: DependencyAction,
    consent_granted: bool,
) -> WireFrame {
    WireFrame::DependencyActionRequest(DependencyActionRequest {
        provider,
        action,
        consent_granted,
    })
}
