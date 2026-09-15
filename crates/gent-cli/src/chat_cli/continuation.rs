use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::AgentChatConversationId;

#[derive(Debug, Args)]
pub(crate) struct ContinueFromHistoryArgs {
    #[arg(long, help = "Conversation whose provider session was lost")]
    pub(crate) conversation_id: String,
    #[arg(long, help = "Message of the failed turn to send again")]
    pub(crate) message_id: String,
    #[arg(long, help = "Client request id used to correlate the reply")]
    pub(crate) request_id: Option<String>,
    #[arg(
        long,
        help = "Receipt id; reuse it to continue the same turn only once [default: derived from the message]"
    )]
    pub(crate) receipt_id: Option<String>,
    #[arg(
        long,
        help = "Print machine-readable JSON frames instead of the streamed reply"
    )]
    pub(crate) json: bool,
}

pub(crate) fn frame(args: ContinueFromHistoryArgs) -> AgentChatIntentFrame {
    AgentChatIntentFrame::ContinueFromSavedHistory {
        request_id: super::request_id(args.request_id),
        receipt_id: gent_types::ReceiptId(
            args.receipt_id
                .unwrap_or_else(|| receipt_for(&args.message_id)),
        ),
        conversation_id: AgentChatConversationId(args.conversation_id),
        message_id: args.message_id,
    }
}

pub(crate) async fn send(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    message_id: String,
) -> Result<super::prompt::PromptAccepted, Box<dyn std::error::Error>> {
    let response = super::exchange(
        data_dir,
        no_autostart,
        frame(ContinueFromHistoryArgs {
            conversation_id,
            message_id,
            request_id: None,
            receipt_id: None,
            json: false,
        }),
    )
    .await?;
    let AgentChatIntentFrame::Accepted {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        receipt,
        ..
    } = response
    else {
        return Err("daemon did not accept the continued prompt".into());
    };
    Ok(super::prompt::PromptAccepted {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        receipt,
    })
}

pub(crate) fn receipt_for(message_id: &str) -> String {
    format!("continue-{message_id}")
}

pub(crate) fn command(prefix: &str, conversation_id: &str, message_id: &str) -> String {
    format!(
        "{prefix} chat continue-from-history --conversation-id {conversation_id} --message-id {message_id}"
    )
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use gent_protocol::AgentChatIntentFrame;

    use crate::{Args, CommandLine, chat_cli::ChatCommand};

    fn parsed(extra: &[&str]) -> AgentChatIntentFrame {
        let mut command_line = vec![
            "gent",
            "chat",
            "continue-from-history",
            "--conversation-id",
            "conversation-1",
            "--message-id",
            "message-7",
        ];
        command_line.extend_from_slice(extra);
        let Some(CommandLine::Chat {
            action: ChatCommand::ContinueFromHistory(args),
        }) = Args::try_parse_from(command_line).unwrap().command
        else {
            panic!("continue-from-history must parse as its own chat command");
        };
        super::frame(args)
    }

    #[test]
    fn repeating_the_continue_command_reuses_one_receipt_so_the_prompt_is_sent_once() {
        let AgentChatIntentFrame::ContinueFromSavedHistory {
            receipt_id,
            conversation_id,
            message_id,
            ..
        } = parsed(&[])
        else {
            panic!("continue-from-history must encode its typed intent");
        };
        assert_eq!(receipt_id.0, "continue-message-7");
        assert_eq!(conversation_id.0, "conversation-1");
        assert_eq!(message_id, "message-7");
        let AgentChatIntentFrame::ContinueFromSavedHistory { receipt_id, .. } = parsed(&[]) else {
            panic!("continue-from-history must encode its typed intent");
        };
        assert_eq!(receipt_id.0, "continue-message-7");
    }
}
