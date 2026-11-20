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

mod commands;
mod correlation;
mod facts;
mod operation_inputs;
mod reasoning;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexTurnEffect {
    Write(Vec<u8>),
    Fact(PublicWireFact),
    ControlRequest(CodexControlRequest),
    Steer(CodexSteerOutcome),
    ProviderTurnStarted,
    SessionState { idle: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexTurnError {
    #[error("a Codex app-server frame exceeded the configured bound")]
    FrameTooLarge,
    #[error(transparent)]
    Session(#[from] CodexSessionError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexTurnDriver {
    session: CodexAppServerSession,
    prompt: Option<String>,
    attachments: Vec<Value>,
    interrupted_reply: Option<String>,
    child_parent_by_thread: BTreeMap<String, String>,
    settled_child_threads: BTreeSet<String>,
    tool_output_item_ids: BTreeSet<String>,
    commands: commands::RunningCommands,
    reasoning: reasoning::ReasoningState,
    operation_inputs: operation_inputs::OperationInputs,
    rate_limit_resets_at: Option<u64>,
}

impl CodexTurnDriver {
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
            Self::new(session, Some(prompt), attachments, interrupted_reply),
            vec![CodexTurnEffect::Write(initialize)],
        ))
    }

    pub fn awaiting_prompt(
        config: CodexSessionConfig,
    ) -> Result<(Self, Vec<CodexTurnEffect>), CodexTurnError> {
        let (session, initialize) = CodexAppServerSession::start(config)?;
        Ok((
            Self::new(session, None, Vec::new(), None),
            vec![CodexTurnEffect::Write(initialize)],
        ))
    }

    fn new(
        session: CodexAppServerSession,
        prompt: Option<String>,
        attachments: Vec<Value>,
        interrupted_reply: Option<String>,
    ) -> Self {
        Self {
            session,
            prompt,
            attachments,
            interrupted_reply,
            child_parent_by_thread: BTreeMap::new(),
            settled_child_threads: BTreeSet::new(),
            tool_output_item_ids: BTreeSet::new(),
            commands: commands::RunningCommands::default(),
            reasoning: reasoning::ReasoningState::default(),
            operation_inputs: operation_inputs::OperationInputs::default(),
            rate_limit_resets_at: None,
        }
    }

    pub(crate) fn bind_external_sandbox(&mut self) -> Result<(), CodexTurnError> {
        self.session.bind_external_sandbox().map_err(Into::into)
    }

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
            Ok(Some(mut request)) => {
                self.operation_inputs.enrich(&frame, &mut request);
                request.child_id = facts::child_thread(
                    &frame,
                    &self.child_parent_by_thread,
                    self.session.thread_id(),
                )
                .map(str::to_owned);
                self.commands.approval_requested(&request.tool_use_id);
                return Ok(vec![CodexTurnEffect::ControlRequest(request)]);
            }
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
        self.operation_inputs.observe(&frame, &mut effects);
        effects.extend(
            facts::root_session_idle(&frame, self.session.thread_id()).map(|idle| {
                CodexTurnEffect::SessionState {
                    idle: self.commands.session_idle(idle) && !self.children_open(),
                }
            }),
        );
        match self.session.receive(&frame) {
            Ok(CodexSessionIngress::Send(frames)) => writes(&mut effects, frames),
            Ok(CodexSessionIngress::Ready { thread_id }) => {
                effects.push(CodexTurnEffect::Fact(PublicWireFact::SessionStarted {
                    provider_session_id: thread_id,
                }));
                if let Some(prompt) = self.prompt.take() {
                    let interrupted_reply = self.interrupted_reply.take();
                    effects.push(CodexTurnEffect::Write(
                        self.session.start_turn_after_interrupted_reply(
                            &prompt,
                            &self.attachments,
                            interrupted_reply.as_deref(),
                        )?,
                    ));
                }
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
            Ok(CodexSessionIngress::ProviderTurnStarted) => {
                effects.insert(0, CodexTurnEffect::ProviderTurnStarted);
            }
            Ok(CodexSessionIngress::TurnStarted | CodexSessionIngress::Ignored) => {}
            Err(_) if notification => {}
            Err(error) => return Err(error.into()),
        }
        Ok(effects)
    }

    pub fn submit(
        &mut self,
        prompt: &str,
        goal: Option<&GoalProjection>,
        attachments: &[Value],
        interrupted_reply: Option<&str>,
    ) -> Result<Vec<CodexTurnEffect>, CodexTurnError> {
        let prompt =
            project_prompt(prompt, goal, 65_536).map_err(|_| CodexSessionError::InvalidPrompt)?;
        if self.session.is_handshaking() {
            CodexAppServerSession::validate_prompt(&prompt)?;
            if self.prompt.is_some() {
                return Err(CodexSessionError::TurnAlreadyActive.into());
            }
            self.prompt = Some(prompt);
            self.attachments = attachments.to_vec();
            self.interrupted_reply = interrupted_reply.map(str::to_owned);
            return Ok(Vec::new());
        }
        Ok(vec![CodexTurnEffect::Write(
            self.session.start_turn_after_interrupted_reply(
                &prompt,
                attachments,
                interrupted_reply,
            )?,
        )])
    }

    #[must_use]
    pub fn steerable(&self) -> bool {
        self.session.steerable()
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

    pub fn approval_answered(&mut self, tool_use_id: &str) {
        self.commands.approval_answered(tool_use_id);
    }

    fn children_open(&self) -> bool {
        self.child_parent_by_thread
            .keys()
            .any(|child| !self.settled_child_threads.contains(child))
    }

    pub fn interrupt(&mut self) -> Result<CodexTurnEffect, CodexTurnError> {
        self.commands.stop();
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
