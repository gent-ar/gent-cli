//! Pure Codex turn bridge: strict app-server handshake plus normalized public facts.
//!
//! The daemon-owned process edge writes only the returned frames and persists only the returned
//! facts. It never receives raw provider fields, native identities, or unbounded output here.

use std::collections::{BTreeMap, BTreeSet};

use gent_types::{
    GoalProjection, NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, TurnPhase,
};
use serde_json::Value;

use crate::PublicProvider;
use crate::codex_client_request::{CodexClientRequestResponse, respond_to_codex_client_request};
use crate::codex_control::{CodexControlRequest, parse as parse_control};
use crate::codex_session::{
    CodexAppServerSession, CodexSessionConfig, CodexSessionError, CodexSessionIngress,
};
use crate::goal_projection::project_prompt;
use crate::public_protocol::{PublicWireFact, normalize_public_frame};

mod facts;

/// Maximum retained Codex app-server line accepted by this driver boundary.
pub const MAX_CODEX_FRAME_BYTES: usize = 64 * 1024;

/// A write or a secret-free normalized fact owned by the daemon process edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexTurnEffect {
    Write(Vec<u8>),
    Fact(PublicWireFact),
    ControlRequest(CodexControlRequest),
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
    child_parent_by_thread: BTreeMap<String, String>,
    settled_child_threads: BTreeSet<String>,
    tool_output_item_ids: BTreeSet<String>,
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
        Self::start_with_attachments(config, prompt, Vec::new(), goal)
    }

    pub fn start_with_attachments(
        config: CodexSessionConfig,
        prompt: &str,
        attachments: Vec<Value>,
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
                attachments,
                child_parent_by_thread: BTreeMap::new(),
                settled_child_threads: BTreeSet::new(),
                tool_output_item_ids: BTreeSet::new(),
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
        match parse_control(&frame) {
            Ok(Some(request)) => return Ok(vec![CodexTurnEffect::ControlRequest(request)]),
            Err(classification) => return Ok(diagnostic(classification)),
            Ok(None) => {}
        }
        let notification = frame.get("method").and_then(Value::as_str).is_some();
        let mut effects = if notification {
            self.facts(&frame)
        } else {
            Vec::new()
        };
        match self.session.receive(&frame) {
            Ok(CodexSessionIngress::Send(frames)) => writes(&mut effects, frames),
            Ok(CodexSessionIngress::Ready { .. }) => {
                let prompt = self
                    .prompt
                    .take()
                    .ok_or(CodexSessionError::TurnAlreadyActive)?;
                effects.push(CodexTurnEffect::Write(
                    self.session
                        .start_turn_with_attachments(&prompt, &self.attachments)?,
                ));
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
        attachments: &[Value],
    ) -> Result<Vec<CodexTurnEffect>, CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        Ok(vec![CodexTurnEffect::Write(
            self.session
                .start_turn_with_attachments(&prompt, attachments)?,
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

impl CodexTurnDriver {
    fn facts(&mut self, frame: &Value) -> Vec<CodexTurnEffect> {
        let terminal = facts::child_terminal(frame, &self.child_parent_by_thread);
        let child_phase = facts::child_phase(frame, &self.child_parent_by_thread);
        let mut facts = if facts::is_empty_turn_completion(frame) {
            self.session.active_turn_id().map_or_else(
                || normalize_public_frame(PublicProvider::Codex, frame),
                |turn_id| {
                    vec![
                        PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
                            turn_id: turn_id.into(),
                        }),
                        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
                            activity: RootActivity::Idle,
                        }),
                        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase {
                            phase: TurnPhase::Ready,
                        }),
                    ]
                },
            )
        } else {
            normalize_public_frame(PublicProvider::Codex, frame)
        };
        // A child turn completion is not a root turn completion. The public
        // normalizer is intentionally stateless, so this owner-side correlation
        // removes the root-only terminal facts once an explicit child mapping
        // proves the frame belongs to detached work.
        if terminal.is_some()
            && matches!(
                facts::method(frame),
                Some("turn/completed" | "turn/failed" | "turn/aborted")
            )
        {
            facts.retain(|fact| !facts::root_terminal_fact(fact));
        }
        for fact in &facts {
            if let PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id,
                ..
            }) = fact
            {
                self.tool_output_item_ids.insert(tool_use_id.clone());
            }
            if let PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
                child_id,
                parent_tool_use_id,
            }) = fact
            {
                self.child_parent_by_thread
                    .entry(child_id.clone())
                    .or_insert_with(|| parent_tool_use_id.clone());
            }
        }
        if let Some(fallback) = self.command_completion_fallback(frame) {
            facts.push(fallback);
        }
        if let Some((child_id, phase)) = child_phase
            && !matches!(
                phase,
                gent_types::WorkPhase::Done
                    | gent_types::WorkPhase::Failed
                    | gent_types::WorkPhase::Interrupted
            )
        {
            facts.push(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ChildPhase { child_id, phase },
            ));
        }
        let mut effects: Vec<_> = facts.into_iter().map(CodexTurnEffect::Fact).collect();
        if let Some((child_id, phase)) = terminal
            && self.settled_child_threads.insert(child_id.clone())
        {
            effects.push(CodexTurnEffect::Fact(PublicWireFact::Event(
                NormalizedProviderEvent::ChildTerminal { child_id, phase },
            )));
        }
        effects
    }

    fn command_completion_fallback(&mut self, frame: &Value) -> Option<PublicWireFact> {
        if facts::method(frame) != Some("item/completed") {
            return None;
        }
        let item = frame.pointer("/params/item")?;
        if item.get("type").and_then(Value::as_str) != Some("commandExecution") {
            return None;
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())?;
        if self.tool_output_item_ids.contains(id) {
            return None;
        }
        let text = item
            .get("aggregatedOutput")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())?;
        self.tool_output_item_ids.insert(id.into());
        Some(PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: id.into(),
                text: text.into(),
                is_partial: false,
            },
        ))
    }
}
fn diagnostic(classification: &str) -> Vec<CodexTurnEffect> {
    vec![CodexTurnEffect::Fact(PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    ))]
}
