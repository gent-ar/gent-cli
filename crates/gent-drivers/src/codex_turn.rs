//! Pure Codex turn bridge: strict app-server handshake plus normalized public facts.
//!
//! The daemon-owned process edge writes only the returned frames and persists only the returned
//! facts. It never receives raw provider fields, native identities, or unbounded output here.

use std::collections::{BTreeMap, BTreeSet};

use gent_types::{GoalProjection, NormalizedProviderEvent};
use serde_json::Value;

use crate::codex_client_request::{
    CodexClientRequestResponse, reject_unhandled_codex_request, respond_to_codex_client_request,
};
use crate::codex_control::{CodexControlRequest, parse as parse_control};
use crate::codex_session::{
    CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
    CodexSteerOutcome,
};
use crate::goal_projection::project_prompt;
use crate::public_protocol::PublicWireFact;

mod correlation;
mod facts;

/// A write or a secret-free normalized fact owned by the daemon process edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexTurnEffect {
    Write(Vec<u8>),
    Fact(PublicWireFact),
    ControlRequest(CodexControlRequest),
    Steer(CodexSteerOutcome),
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
    attachments: Vec<Value>,
    interrupted_reply: Option<String>,
    child_parent_by_thread: BTreeMap<String, String>,
    settled_child_threads: BTreeSet<String>,
    tool_output_item_ids: BTreeSet<String>,
    reported_root_failure: bool,
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
        Self::start_with_attachments(config, prompt, Vec::new(), goal, None)
    }

    pub fn start_with_attachments(
        config: CodexSessionConfig,
        prompt: &str,
        attachments: Vec<Value>,
        goal: Option<&GoalProjection>,
        interrupted_reply: Option<String>,
    ) -> Result<(Self, Vec<CodexTurnEffect>), CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        CodexAppServerSession::validate_prompt(&prompt)?;
        let (session, initialize) = CodexAppServerSession::start(config)?;
        Ok((
            Self {
                session,
                prompt: Some(prompt),
                attachments,
                interrupted_reply,
                child_parent_by_thread: BTreeMap::new(),
                settled_child_threads: BTreeSet::new(),
                tool_output_item_ids: BTreeSet::new(),
                reported_root_failure: false,
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
        if raw.len() > crate::MAX_PROVIDER_FRAME_BYTES {
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
        match parse_control(&frame) {
            Ok(Some(request)) => return Ok(vec![CodexTurnEffect::ControlRequest(request)]),
            Err(classification) => return Ok(rejected(&frame, classification)),
            Ok(None) => {}
        }
        if frame.get("id").is_some() && frame.get("method").is_some() {
            return Ok(rejected(&frame, "unsupportedCodexServerRequest"));
        }
        let notification = frame.get("method").and_then(Value::as_str).is_some();
        let mut effects = if notification {
            self.facts(&frame)
        } else {
            Vec::new()
        };
        match self.session.receive(&frame) {
            Ok(CodexSessionIngress::Send(frames)) => writes(&mut effects, frames),
            Ok(CodexSessionIngress::Ready { thread_id }) => {
                // The correlated response is the sole authoritative thread identity.
                // `thread/started` is an asynchronous notification and may arrive
                // before or after this response while reconnecting.
                effects.push(CodexTurnEffect::Fact(PublicWireFact::SessionStarted {
                    provider_session_id: thread_id,
                }));
                let prompt = self
                    .prompt
                    .take()
                    .ok_or(CodexSessionError::TurnAlreadyActive)?;
                let interrupted_reply = self.interrupted_reply.take();
                effects.push(CodexTurnEffect::Write(
                    self.session.start_turn_after_interrupted_reply(
                        &prompt,
                        &self.attachments,
                        interrupted_reply.as_deref(),
                    )?,
                ));
            }
            Ok(CodexSessionIngress::Steer(outcome)) => {
                effects.push(CodexTurnEffect::Steer(outcome))
            }
            Ok(CodexSessionIngress::TurnEnded) => effects.extend(
                self.session
                    .unconsumed_steers()
                    .into_iter()
                    .map(CodexTurnEffect::Steer),
            ),
            Ok(CodexSessionIngress::TurnStarted | CodexSessionIngress::Ignored) => {}
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
        attachments: &[Value],
        interrupted_reply: Option<&str>,
    ) -> Result<Vec<CodexTurnEffect>, CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        Ok(vec![CodexTurnEffect::Write(
            self.session.start_turn_after_interrupted_reply(
                &prompt,
                attachments,
                interrupted_reply,
            )?,
        )])
    }

    pub fn steer(
        &mut self,
        message_id: &str,
        prompt: &str,
        attachments: &[Value],
    ) -> Result<CodexTurnEffect, CodexTurnError> {
        Ok(CodexTurnEffect::Write(self.session.steer(
            message_id,
            prompt,
            attachments,
        )?))
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

fn rejected(frame: &Value, classification: &str) -> Vec<CodexTurnEffect> {
    let mut effects = diagnostic(classification);
    effects.extend(reject_unhandled_codex_request(frame).map(CodexTurnEffect::Write));
    effects
}

fn diagnostic(classification: &str) -> Vec<CodexTurnEffect> {
    vec![CodexTurnEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}
