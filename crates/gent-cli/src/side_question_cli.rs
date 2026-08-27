use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{AGENT_CHAT_SIDE_QUESTION_CAPABILITY, AgentChatSideQuestionFrame};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub(crate) enum SideQuestionCommand {
    Ask {
        #[arg(long)]
        conversation_id: String,
        question: String,
    },
    Cancel {
        #[arg(long)]
        side_question_id: String,
    },
    List {
        #[arg(long)]
        conversation_id: String,
    },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: SideQuestionCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let frame = match command {
        SideQuestionCommand::Ask {
            conversation_id,
            question,
        } => AgentChatSideQuestionFrame::AskSideQuestion {
            request_id: uuid::Uuid::new_v4().to_string(),
            receipt_id: uuid::Uuid::new_v4().to_string(),
            conversation_id,
            question,
        },
        SideQuestionCommand::Cancel { side_question_id } => {
            AgentChatSideQuestionFrame::CancelSideQuestion {
                request_id: uuid::Uuid::new_v4().to_string(),
                receipt_id: uuid::Uuid::new_v4().to_string(),
                side_question_id,
            }
        }
        SideQuestionCommand::List { conversation_id } => {
            AgentChatSideQuestionFrame::ListSideQuestions {
                request_id: uuid::Uuid::new_v4().to_string(),
                conversation_id,
            }
        }
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == AGENT_CHAT_SIDE_QUESTION_CAPABILITY)
    {
        return Err("agent chat side question capability is unavailable".into());
    }
    gent_protocol::write_json_frame(&mut stream, &frame).await?;
    let reply: AgentChatSideQuestionFrame = gent_protocol::read_json_frame(&mut stream).await?;
    Ok(serde_json::to_value(reply)?)
}
