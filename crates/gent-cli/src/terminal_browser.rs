//! Terminal composition edge: it connects UI requests to protocol-only agent-chat IPC.

use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
};
use gent_types::{AgentChatPromptDelivery, ConversationListItem};

use crate::{chat_cli, conversation_index, conversation_status, local_ipc, terminal};

/// Opens the terminal after negotiation has established the available authority profile.
pub(crate) async fn open(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::require_interactive()?;
    let index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    let (_, capabilities) =
        local_ipc::connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    let enabled = capabilities
        .0
        .iter()
        .any(|value| value == AGENT_CHAT_INTENTS_CAPABILITY);
    let view = initial_view(&index, &capabilities.0, data_dir.clone(), no_autostart).await;
    let runtime = tokio::runtime::Handle::current();
    let request_runtime = runtime.clone();
    let request_data_dir = data_dir.clone();
    let view_runtime = runtime.clone();
    let view_data_dir = data_dir.clone();
    let view_capabilities = capabilities.0.clone();
    terminal::run(
        terminal::UiState::new(index)
            .with_chat_input(enabled)
            .with_view(view),
        move |intent| {
            submit(
                &request_runtime,
                request_data_dir.clone(),
                no_autostart,
                intent,
            )
        },
        move |conversation_id| {
            tokio::task::block_in_place(|| {
                view_runtime.block_on(read_view(
                    view_data_dir.clone(),
                    no_autostart,
                    conversation_id,
                    &view_capabilities,
                ))
            })
        },
    )?;
    Ok(())
}

async fn initial_view(
    index: &[ConversationListItem],
    capabilities: &[String],
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Option<terminal::ConversationView> {
    let conversation_id = index.first()?.conversation_id.clone();
    read_view(data_dir, no_autostart, conversation_id, capabilities)
        .await
        .ok()
}

async fn read_view(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    capabilities: &[String],
) -> Result<terminal::ConversationView, String> {
    let status = if capabilities
        .iter()
        .any(|value| value == CONVERSATION_STATUS_CAPABILITY)
    {
        Some(
            conversation_status::request(data_dir.clone(), no_autostart, conversation_id.clone())
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let transcript = if capabilities
        .iter()
        .any(|value| value == AGENT_CHAT_TRANSCRIPT_CAPABILITY)
    {
        Some(
            chat_cli::transcript(data_dir, no_autostart, conversation_id.clone(), None, 100)
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    Ok(terminal::ConversationView::new(
        conversation_id,
        status,
        transcript,
    ))
}

fn submit(
    runtime: &tokio::runtime::Handle,
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: terminal::UiRequest,
) -> Result<terminal::UiRequestResult, String> {
    tokio::task::block_in_place(|| {
        runtime.block_on(async move {
            match request {
                terminal::UiRequest::Create { selection } => {
                    let (conversation_id, run_id) =
                        chat_cli::create(data_dir, no_autostart, selection, None)
                            .await
                            .map_err(|error| error.to_string())?;
                    Ok(result(
                        conversation_id.0,
                        Some(run_id.0),
                        "Conversation created; choose a prompt to persist.",
                    ))
                }
                terminal::UiRequest::Send {
                    conversation_id,
                    text,
                } => {
                    let accepted =
                        chat_cli::send(data_dir, no_autostart, conversation_id.clone(), text)
                            .await
                            .map_err(|error| error.to_string())?;
                    Ok(result(
                        accepted.conversation_id.0,
                        Some(accepted.run_id.0),
                        delivery_notice(accepted.delivery),
                    ))
                }
                terminal::UiRequest::Goal {
                    conversation_id,
                    run_id,
                    summary,
                } => {
                    crate::goal_cli::create_shorthand(
                        data_dir,
                        no_autostart,
                        conversation_id.clone(),
                        run_id,
                        summary,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    Ok(result(
                        conversation_id,
                        None,
                        "Goal saved; it will be projected only by an authorized provider turn.",
                    ))
                }
                terminal::UiRequest::Switch {
                    conversation_id,
                    parent_run_id,
                    selection,
                    context_policy,
                } => {
                    let run_id = chat_cli::switch::request(
                        data_dir,
                        no_autostart,
                        conversation_id.clone(),
                        parent_run_id,
                        selection,
                        context_policy,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    Ok(result(
                        conversation_id,
                        Some(run_id.0),
                        "Selection switched; prompts now target the new durable run.",
                    ))
                }
            }
        })
    })
}

fn result(
    conversation_id: String,
    parent_run_id: Option<String>,
    notice: impl Into<String>,
) -> terminal::UiRequestResult {
    terminal::UiRequestResult {
        conversation: ConversationListItem {
            conversation_id,
            run_count: 1,
        },
        parent_run_id,
        notice: notice.into(),
    }
}

const fn delivery_notice(delivery: AgentChatPromptDelivery) -> &'static str {
    match delivery {
        AgentChatPromptDelivery::Queued => {
            "Prompt queued locally; no provider delivery was attempted."
        }
        AgentChatPromptDelivery::AwaitingReadiness => {
            "Prompt is durable and awaiting Gent's verified provider readiness review."
        }
        AgentChatPromptDelivery::AwaitingProvider => {
            "Prompt is durable and awaiting an authorized provider lifecycle."
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{AgentChatPromptDelivery, ConversationListItem};

    use super::{delivery_notice, initial_view};

    #[test]
    fn prompt_delivery_notice_never_claims_a_provider_started() {
        assert!(delivery_notice(AgentChatPromptDelivery::Queued).contains("no provider"));
        assert!(delivery_notice(AgentChatPromptDelivery::AwaitingReadiness).contains("readiness"));
        assert!(delivery_notice(AgentChatPromptDelivery::AwaitingProvider).contains("awaiting"));
    }

    #[tokio::test]
    async fn initial_view_does_not_infer_activity_or_content_without_capabilities() {
        let index = vec![ConversationListItem {
            conversation_id: "conversation-1".into(),
            run_count: 1,
        }];
        assert!(initial_view(&index, &[], None, true).await.is_none());
    }
}
