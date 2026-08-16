//! Typed `gent chat` requests over negotiated local agent-chat IPC.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, ReceiptId,
};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

#[derive(Debug, Subcommand)]
pub(crate) enum ChatCommand {
    Create(CreateArgs),
    Send(PromptArgs),
    Queue(PromptArgs),
}

#[derive(Debug, Args)]
pub(crate) struct CreateArgs {
    #[arg(long, value_enum)]
    provider: Provider,
    #[arg(long)]
    model: String,
    #[arg(long, value_enum, default_value_t = Effort::Medium)]
    effort: Effort,
    #[arg(long, value_enum, default_value_t = Mode::Ask)]
    mode: Mode,
    #[arg(long)]
    request_id: Option<String>,
    #[arg(long)]
    receipt_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PromptArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    text: String,
    #[arg(long)]
    request_id: Option<String>,
    #[arg(long)]
    receipt_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Provider {
    Claude,
    Codex,
    Claurst,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Effort {
    Low,
    #[default]
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Mode {
    #[default]
    Ask,
    Plan,
    Agent,
}

/// Exchanges exactly one capability-gated agent-chat intent with the local daemon.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: ChatCommand,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    let request = frame(action);
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_INTENTS_CAPABILITY)
    {
        return Err("daemon does not support agent chat; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response =
        serde_json::from_value(raw).map_err(|_| "daemon did not return an agent-chat response")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned an agent-chat response with a different request or receipt".into()
        })
}

fn frame(action: ChatCommand) -> AgentChatIntentFrame {
    match action {
        ChatCommand::Create(args) => AgentChatIntentFrame::CreateConversation {
            request_id: request_id(args.request_id),
            receipt_id: receipt_id(args.receipt_id),
            selection: AgentChatSelection {
                provider: provider(args.provider),
                model: args.model,
                effort: effort(args.effort),
                mode: mode(args.mode),
            },
        },
        ChatCommand::Send(args) => prompt_frame(args, false),
        ChatCommand::Queue(args) => prompt_frame(args, true),
    }
}

fn prompt_frame(args: PromptArgs, queued: bool) -> AgentChatIntentFrame {
    let value = (
        request_id(args.request_id),
        receipt_id(args.receipt_id),
        AgentChatConversationId(args.conversation_id),
        args.text,
    );
    if queued {
        AgentChatIntentFrame::QueuePrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
        }
    } else {
        AgentChatIntentFrame::SendPrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
        }
    }
}

fn valid_reply(request: &AgentChatIntentFrame, response: &AgentChatIntentFrame) -> bool {
    match (request, response) {
        (
            AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                ..
            },
            AgentChatIntentFrame::Created {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
        }
        (
            AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                ..
            }
            | AgentChatIntentFrame::QueuePrompt {
                request_id,
                receipt_id,
                ..
            },
            AgentChatIntentFrame::Accepted {
                request_id: reply,
                receipt,
            },
        ) => reply == request_id && receipt.receipt_id == *receipt_id,
        _ => false,
    }
}

fn request_id(value: Option<String>) -> AgentChatRequestId {
    AgentChatRequestId(value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()))
}
fn receipt_id(value: Option<String>) -> ReceiptId {
    value.map_or_else(ReceiptId::new, ReceiptId)
}
const fn provider(value: Provider) -> AgentChatProvider {
    match value {
        Provider::Claude => AgentChatProvider::Claude,
        Provider::Codex => AgentChatProvider::Codex,
        Provider::Claurst => AgentChatProvider::Claurst,
    }
}
const fn effort(value: Effort) -> AgentChatEffort {
    match value {
        Effort::Low => AgentChatEffort::Low,
        Effort::Medium => AgentChatEffort::Medium,
        Effort::High => AgentChatEffort::High,
    }
}
const fn mode(value: Mode) -> AgentChatMode {
    match value {
        Mode::Ask => AgentChatMode::Ask,
        Mode::Plan => AgentChatMode::Plan,
        Mode::Agent => AgentChatMode::Agent,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use gent_protocol::{
        AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Hello, Negotiated, WireFrame,
        read_frame, read_json_frame, write_frame, write_json_frame,
    };
    use gent_types::{
        AgentChatConversationId, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
    };
    use tokio::net::UnixListener;

    use super::{ChatCommand, CreateArgs, Effort, Mode, Provider, execute};

    #[tokio::test]
    async fn create_negotiates_agent_chat_and_requires_a_matching_created_reply() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_frame(&mut stream).await.unwrap(),
                WireFrame::Hello(Hello { capabilities, .. })
                    if capabilities.0.iter().any(|item| item == AGENT_CHAT_INTENTS_CAPABILITY)
            ));
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()]),
                }),
            )
            .await
            .unwrap();
            let AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                ..
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected create");
            };
            write_json_frame(
                &mut stream,
                &AgentChatIntentFrame::Created {
                    request_id,
                    receipt: Receipt {
                        receipt_id,
                        idempotency_key: "redacted".into(),
                        status: ReceiptStatus::Settled,
                        host_epoch: HostEpoch(1),
                    },
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: gent_types::AgentChatRunId("run-1".into()),
                },
            )
            .await
            .unwrap();
        });
        let reply = execute(
            Some(directory.path().into()),
            true,
            ChatCommand::Create(CreateArgs {
                provider: Provider::Claude,
                model: "haiku".into(),
                effort: Effort::Low,
                mode: Mode::Ask,
                request_id: Some("request-1".into()),
                receipt_id: Some("receipt-1".into()),
            }),
        )
        .await
        .unwrap();
        assert!(
            matches!(reply, AgentChatIntentFrame::Created { conversation_id, run_id, .. } if conversation_id.0 == "conversation-1" && run_id.0 == "run-1")
        );
    }
}
