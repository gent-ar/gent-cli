//! Pure Codex turn bridge: strict app-server handshake plus normalized public facts.
//!
//! The daemon-owned process edge writes only the returned frames and persists only the returned
//! facts. It never receives raw provider fields, native identities, or unbounded output here.

use gent_types::{GoalProjection, NormalizedProviderEvent};
use serde_json::Value;

use crate::PublicProvider;
use crate::codex_client_request::{CodexClientRequestResponse, respond_to_codex_client_request};
use crate::codex_session::{
    CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
};
use crate::goal_projection::project_prompt;
use crate::public_protocol::{PublicWireFact, normalize_public_frame};

/// Maximum retained Codex app-server line accepted by this driver boundary.
pub const MAX_CODEX_FRAME_BYTES: usize = 64 * 1024;

/// A write or a secret-free normalized fact owned by the daemon process edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexTurnEffect {
    /// Write exactly this newline-delimited JSON-RPC frame to the owned process.
    Write(Vec<u8>),
    /// Persist or project only this provider-neutral public fact.
    Fact(PublicWireFact),
}

/// Controlled failure while correlating a Codex app-server response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexTurnError {
    #[error("a Codex app-server frame exceeded the configured bound")]
    FrameTooLarge,
    #[error(transparent)]
    Session(#[from] CodexSessionError),
}

/// One prompt held only until its initial `turn/start` frame is encoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTurnDriver {
    session: CodexAppServerSession,
    prompt: Option<String>,
}

impl CodexTurnDriver {
    /// Starts the bounded handshake without launching, reading, or persisting anything.
    ///
    /// # Errors
    /// Rejects invalid native configuration or an invalid prompt before a process may be started.
    pub fn start(
        config: CodexSessionConfig,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<(Self, Vec<CodexTurnEffect>), CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        CodexAppServerSession::validate_prompt(&prompt)?;
        let (session, initialize) = CodexAppServerSession::start(config)?;
        Ok((
            Self {
                session,
                prompt: Some(prompt),
            },
            vec![CodexTurnEffect::Write(initialize)],
        ))
    }

    /// Reduces one bounded JSON-RPC frame into process writes and normalized public facts.
    ///
    /// Provider notifications are normalized even when irrelevant to handshake state. Malformed
    /// notifications become diagnostics and do not poison the pending request correlation.
    ///
    /// # Errors
    /// Returns only for oversized input or an invalid correlated response; no raw payload is kept.
    pub fn receive(&mut self, raw: &[u8]) -> Result<Vec<CodexTurnEffect>, CodexTurnError> {
        if raw.len() > MAX_CODEX_FRAME_BYTES {
            return Err(CodexTurnError::FrameTooLarge);
        }
        let Ok(frame) = serde_json::from_slice::<Value>(raw) else {
            return Ok(diagnostic("malformedCodexFrame"));
        };
        match respond_to_codex_client_request(&frame, epoch_seconds()) {
            CodexClientRequestResponse::Write(response) => {
                return Ok(vec![CodexTurnEffect::Write(response)]);
            }
            CodexClientRequestResponse::Malformed => {
                return Ok(diagnostic("malformedCodexClientRequest"));
            }
            CodexClientRequestResponse::NotHandled => {}
        }
        let notification = frame.get("method").and_then(Value::as_str).is_some();
        let mut effects = if notification {
            facts(&frame)
        } else {
            Vec::new()
        };
        match self.session.receive(&frame) {
            Ok(CodexSessionIngress::Send(frames)) => writes(&mut effects, frames),
            Ok(CodexSessionIngress::Ready) => {
                let prompt = self
                    .prompt
                    .take()
                    .ok_or(CodexSessionError::TurnAlreadyActive)?;
                effects.push(CodexTurnEffect::Write(self.session.start_turn(&prompt)?));
            }
            Ok(
                CodexSessionIngress::TurnStarted
                | CodexSessionIngress::TurnEnded
                | CodexSessionIngress::Ignored,
            ) => {}
            Err(_) if notification => {}
            Err(error) => return Err(error.into()),
        }
        Ok(effects)
    }

    /// Encodes one later user turn with a freshly ledger-resolved active goal on the ready thread.
    ///
    /// # Errors
    /// Returns an error until the previous turn is terminal or when the prompt is invalid.
    pub fn submit(
        &mut self,
        prompt: &str,
        goal: Option<&GoalProjection>,
    ) -> Result<Vec<CodexTurnEffect>, CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        Ok(vec![CodexTurnEffect::Write(
            self.session.start_turn(&prompt)?,
        )])
    }

    /// Requests a documented Codex turn interruption without destroying the owned session.
    ///
    /// # Errors
    /// Rejects the request unless the exact turn is live.
    pub fn interrupt(&mut self) -> Result<CodexTurnEffect, CodexTurnError> {
        Ok(CodexTurnEffect::Write(self.session.interrupt()?))
    }
}

fn epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn writes(effects: &mut Vec<CodexTurnEffect>, frames: Vec<Vec<u8>>) {
    effects.extend(frames.into_iter().map(CodexTurnEffect::Write));
}

fn facts(frame: &Value) -> Vec<CodexTurnEffect> {
    normalize_public_frame(PublicProvider::Codex, frame)
        .into_iter()
        .map(CodexTurnEffect::Fact)
        .collect()
}

fn diagnostic(classification: &str) -> Vec<CodexTurnEffect> {
    vec![CodexTurnEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}
