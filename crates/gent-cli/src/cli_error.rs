use std::{error::Error, fmt, io, process::ExitCode};

use gent_protocol::WireFrame;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Rejected,
    Unavailable,
    NotFound,
    ConsentRequired,
    TurnFailed,
    TurnInterrupted,
}

impl Failure {
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Self::Rejected => 1,
            Self::Unavailable => 3,
            Self::NotFound => 4,
            Self::ConsentRequired => 5,
            Self::TurnFailed => 6,
            Self::TurnInterrupted => 7,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CliError {
    failure: Failure,
    message: String,
    code: Option<String>,
}

impl CliError {
    pub(crate) fn new(failure: Failure, message: impl Into<String>) -> Self {
        Self {
            failure,
            message: message.into(),
            code: None,
        }
    }

    pub(crate) fn daemon(code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = Some(code.into()).filter(|code| !code.is_empty());
        Self {
            failure: code.as_deref().map_or(Failure::Rejected, daemon_failure),
            message: message.into(),
            code,
        }
    }

    pub(crate) fn from_reply(raw: &Value) -> Option<Self> {
        match serde_json::from_value(raw.clone()) {
            Ok(WireFrame::Error { code, message }) => Some(Self::daemon(code, message)),
            _ => None,
        }
    }

    pub(crate) const fn failure(&self) -> Failure {
        self.failure
    }

    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code() {
            Some(code) => write!(formatter, "{} [{code}]", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

impl Error for CliError {}

fn daemon_failure(code: &str) -> Failure {
    match code {
        "conversationNotFound" | "workspaceNotFound" => Failure::NotFound,
        _ => Failure::Rejected,
    }
}

pub(crate) fn describe(error: &(dyn Error + 'static)) -> (Failure, String) {
    if let Some(error) = error.downcast_ref::<CliError>() {
        return (error.failure(), error.to_string());
    }
    if let Some(error) = error.downcast_ref::<io::Error>() {
        return (Failure::Rejected, io_message(error));
    }
    (Failure::Rejected, error.to_string())
}

fn io_message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => "gentd closed the connection before replying".into(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe => {
            "the connection to gentd was lost".into()
        }
        _ => error.to_string(),
    }
}

pub(crate) fn report(error: &(dyn Error + 'static)) -> ExitCode {
    let (failure, message) = describe(error);
    eprintln!("gent: {message}");
    ExitCode::from(failure.exit_code())
}

#[cfg(test)]
#[path = "cli_error_tests.rs"]
mod tests;
