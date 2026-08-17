//! CLI-only argument DTOs for provider-neutral chat intents and reads.

use clap::{Args, ValueEnum};

#[derive(Debug, Args)]
pub(crate) struct CreateArgs {
    #[arg(long, value_enum)]
    pub(crate) provider: Provider,
    #[arg(long)]
    pub(crate) model: String,
    #[arg(long, value_enum, default_value_t = Effort::Medium)]
    pub(crate) effort: Effort,
    #[arg(long, value_enum, default_value_t = Mode::Ask)]
    pub(crate) mode: Mode,
    #[arg(long)]
    pub(crate) request_id: Option<String>,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PromptArgs {
    #[arg(long)]
    pub(crate) conversation_id: String,
    #[arg(long)]
    pub(crate) text: String,
    #[arg(long)]
    pub(crate) request_id: Option<String>,
    #[arg(long)]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ConversationArgs {
    #[arg(long)]
    pub(crate) conversation_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct TranscriptArgs {
    #[arg(long)]
    pub(crate) conversation_id: String,
    /// Read strictly after this durable normalized transcript cursor.
    #[arg(long)]
    pub(crate) after_cursor: Option<u64>,
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u16,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Provider {
    Claude,
    Codex,
    Claurst,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Effort {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Mode {
    #[default]
    Ask,
    Plan,
    Agent,
}
