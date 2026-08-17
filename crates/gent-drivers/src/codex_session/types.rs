//! Public bounded inputs, consequences, and failures for a Codex session.

/// Inputs fixed before the Codex app-server initialization request is sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexSessionConfig {
    /// Optional provider working directory for the thread request.
    pub working_directory: Option<String>,
    /// The exact previously recorded native thread to reattach, if any.
    pub resume_thread_id: Option<String>,
}

/// Public consequences of one app-server frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSessionIngress {
    /// Frames that must be written in order before reading another provider response.
    Send(Vec<Vec<u8>>),
    /// The matching thread response established a usable native thread.
    Ready,
    /// The matching turn response established a live native turn.
    TurnStarted,
    /// The live native turn ended.
    TurnEnded,
    /// The frame did not affect this session.
    Ignored,
}

/// Controlled failures for the bounded Codex app-server handshake.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexSessionError {
    #[error("the Codex working directory must be non-empty and within the driver limit")]
    InvalidWorkingDirectory,
    #[error("the Codex native thread identity must be non-empty and within the driver limit")]
    InvalidThreadId,
    #[error("a Codex user prompt must be non-empty and within the driver limit")]
    InvalidPrompt,
    #[error("the Codex app-server thread is not ready for a user turn")]
    ThreadNotReady,
    #[error("the Codex app-server already has a live or pending user turn")]
    TurnAlreadyActive,
    #[error("the Codex app-server session previously failed")]
    SessionFailed,
    #[error("the Codex app-server request identifier space is exhausted")]
    RequestIdExhausted,
    #[error("the Codex app-server returned a malformed response")]
    MalformedResponse,
    #[error("the Codex app-server returned an uncorrelated response")]
    UncorrelatedResponse,
    #[error("the Codex app-server rejected a correlated request")]
    RequestRejected,
    #[error("the resumed Codex thread did not match the exact recorded identity")]
    ResumedThreadMismatch,
    #[error("the Codex turn notification and response disagreed")]
    TurnIdentityMismatch,
    #[error("the Codex app-server frame could not be encoded")]
    Serialization,
}
