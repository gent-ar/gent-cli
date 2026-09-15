//! CLI-only argument DTOs for provider-neutral chat intents and reads.

use clap::{ArgGroup, Args, ValueEnum};
use std::path::PathBuf;

use super::SelectionRequest;

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("new_conversation_selection")
        .args(["provider", "model", "effort", "mode"])
        .multiple(true)
        .requires("prompt")
        .conflicts_with("conversation_id")
))]
pub(crate) struct DirectPromptArgs {
    #[arg(
        value_name = "PROMPT",
        help = "Send this prompt and stream the reply; without a prompt, open the terminal client"
    )]
    pub(crate) prompt: Option<String>,
    #[arg(
        long,
        requires = "prompt",
        help = "Continue this existing conversation instead of creating a new one"
    )]
    pub(crate) conversation_id: Option<String>,
    #[arg(
        long,
        requires = "prompt",
        conflicts_with = "conversation_id",
        help = "Workspace directory for a new conversation [default: current directory]"
    )]
    pub(crate) workspace: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) selection: SelectionArgs,
    #[arg(
        long = "attach",
        value_name = "PATH",
        requires = "prompt",
        help = "Attach a local file to the prompt (repeatable)"
    )]
    pub(crate) attachments: Vec<PathBuf>,
    #[arg(
        long,
        requires = "prompt",
        help = "Print machine-readable JSON frames instead of the streamed reply"
    )]
    pub(crate) json: bool,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct SelectionArgs {
    #[arg(
        long,
        value_enum,
        help = "Provider for a new conversation; alone it uses that provider's default model"
    )]
    pub(crate) provider: Option<Provider>,
    #[arg(
        long,
        help = "Model id from Gentd's model catalog [default: the provider's default model]"
    )]
    pub(crate) model: Option<String>,
    #[arg(
        long,
        value_parser = super::selection::parse_effort,
        help = "Reasoning effort the model accepts; gentd rejects any other and lists the model's choices [default: the model's default]"
    )]
    pub(crate) effort: Option<gent_types::AgentChatEffort>,
    #[arg(long, value_enum, help = "Conversation mode [default: agent]")]
    pub(crate) mode: Option<Mode>,
}

impl SelectionArgs {
    pub(crate) fn request(self) -> SelectionRequest {
        SelectionRequest {
            provider: self.provider,
            model: self.model,
            effort: self.effort,
            mode: self.mode,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct CreateArgs {
    #[arg(
        long,
        help = "Workspace directory for the new conversation [default: current directory]"
    )]
    pub(crate) workspace: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) selection: SelectionArgs,
    #[arg(long, help = "Client request id used to correlate the reply")]
    pub(crate) request_id: Option<String>,
    #[arg(long, help = "Receipt id; reuse it to retry the same request safely")]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PromptArgs {
    #[arg(long, help = "Conversation that receives the prompt")]
    pub(crate) conversation_id: String,
    #[arg(long, help = "Prompt text")]
    pub(crate) text: String,
    #[arg(long, help = "Client request id used to correlate the reply")]
    pub(crate) request_id: Option<String>,
    #[arg(long, help = "Receipt id; reuse it to retry the same prompt safely")]
    pub(crate) receipt_id: Option<String>,
    #[arg(
        long = "attach",
        value_name = "PATH",
        help = "Attach a local file to the prompt (repeatable)"
    )]
    pub(crate) attachments: Vec<PathBuf>,
    #[arg(
        long = "tool-source",
        value_name = "ID",
        help = "Limit the prompt to this tool source (repeatable)"
    )]
    pub(crate) tool_sources: Vec<String>,
    #[arg(
        long,
        help = "Print machine-readable JSON frames instead of the streamed reply"
    )]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ConversationArgs {
    #[arg(long, help = "Conversation to read")]
    pub(crate) conversation_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct TranscriptArgs {
    #[arg(long, help = "Conversation to read")]
    pub(crate) conversation_id: String,
    #[arg(long, help = "Read only events after this transcript cursor")]
    pub(crate) after_cursor: Option<u64>,
    #[arg(long, default_value_t = 50, help = "Maximum events to return")]
    pub(crate) limit: u16,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Provider {
    Claude,
    Codex,
    Gent,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Mode {
    #[default]
    Ask,
    Plan,
    Agent,
}
