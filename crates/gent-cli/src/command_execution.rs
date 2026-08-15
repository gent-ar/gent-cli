//! CLI composition root: maps parsed commands to the protocol-only local IPC boundary.

use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, WireFrame,
};
use gent_types::{Command, ReceiptId};
use serde_json::Value;

use crate::decision::decision_frame;
use crate::local_ipc::request;
use crate::{
    Args, CommandLine, ConversationCommand, DependencyCommand, conversation_index,
    conversation_status, conversation_timeline, event_stream, terminal,
};

pub(crate) async fn execute(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let Args {
        data_dir,
        no_autostart,
        conversations,
        command,
    } = args;
    if conversations && command.is_some() {
        return Err("--conversations cannot be combined with a subcommand".into());
    }
    if conversations || command.is_none() {
        let index = conversation_index::request(data_dir, no_autostart).await?;
        return Ok(terminal::run(terminal::UiState::new(index))?);
    }
    let command = command.expect("conversation browser handles no-subcommand invocation");
    match command {
        CommandLine::Doctor => {
            print(request(data_dir, no_autostart, WireFrame::DoctorRequest).await?)?;
        }
        CommandLine::Onboarding => {
            print(request(data_dir, no_autostart, WireFrame::OnboardingRequest).await?)?;
        }
        CommandLine::Deps { action } => {
            dependency(data_dir, no_autostart, action).await?;
        }
        CommandLine::Decision { action } => {
            print(request(data_dir, no_autostart, decision_frame(&action)).await?)?;
        }
        CommandLine::Conversation { action } => match action {
            ConversationCommand::List => {
                print(conversation_index::request(data_dir, no_autostart).await?)?;
            }
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

async fn dependency(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    command: DependencyCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        DependencyCommand::Plan { action, provider } => print(
            request(
                data_dir,
                no_autostart,
                dependency_plan_frame(provider, action),
            )
            .await?,
        ),
        DependencyCommand::Install {
            provider,
            consent,
            idempotency_key,
        } => {
            dependency_action(
                data_dir,
                no_autostart,
                provider,
                DependencyAction::Install,
                consent,
                idempotency_key,
            )
            .await
        }
        DependencyCommand::Update {
            provider,
            consent,
            idempotency_key,
        } => {
            dependency_action(
                data_dir,
                no_autostart,
                provider,
                DependencyAction::Update,
                consent,
                idempotency_key,
            )
            .await
        }
    }
}

pub(crate) fn dependency_plan_frame(
    provider: DependencyProvider,
    action: DependencyAction,
) -> WireFrame {
    WireFrame::DependencyPlanRequest(DependencyPlanRequest { provider, action })
}

async fn dependency_action(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    provider: DependencyProvider,
    action: DependencyAction,
    consent_granted: bool,
    idempotency_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = request(
        data_dir.clone(),
        no_autostart,
        dependency_plan_frame(provider, action),
    )
    .await?;
    let WireFrame::DependencyPlan(plan) = plan else {
        return Err("daemon did not return a dependency plan".into());
    };
    let status = request(data_dir.clone(), no_autostart, WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = status else {
        return Err("daemon did not return host status".into());
    };
    let action = DependencyActionRequest {
        provider,
        action,
        consent_granted,
        receipt_id: ReceiptId::new(),
        idempotency_key: idempotency_key.unwrap_or_else(|| ReceiptId::new().0),
        host_epoch: status.host_epoch,
        reviewed_plan_digest: plan.reviewed_plan_digest,
    };
    print(
        request(
            data_dir,
            no_autostart,
            WireFrame::DependencyActionRequest(action),
        )
        .await?,
    )
}
