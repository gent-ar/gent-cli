use std::collections::BTreeSet;

use gent_protocol::LocalModelInstallState;
use gent_types::{
    AgentChatProvider, DurableTurnPhase, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    PermissionDecisionRequest, TurnTerminalCause,
};

use crate::cli_error::{CliError, Failure};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Output {
    Reply(String),
    Status(String),
}

#[derive(Clone, Debug)]
pub(crate) struct TurnView {
    command_prefix: String,
    conversation_id: String,
    run_id: String,
    live_progress: bool,
    streamed: String,
    reply_open: bool,
    progress_open: bool,
    progress_percent: Option<u64>,
    user_messages: usize,
    last_notice: Option<String>,
    message_id: Option<String>,
    planned: bool,
    announced: BTreeSet<String>,
    outputs: Vec<Output>,
}

impl TurnView {
    pub(crate) fn new(command_prefix: &str, conversation_id: &str, run_id: &str) -> Self {
        Self {
            command_prefix: command_prefix.to_owned(),
            conversation_id: conversation_id.to_owned(),
            run_id: run_id.to_owned(),
            live_progress: false,
            streamed: String::new(),
            reply_open: false,
            progress_open: false,
            progress_percent: None,
            user_messages: 0,
            last_notice: None,
            message_id: None,
            planned: false,
            announced: BTreeSet::new(),
            outputs: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) const fn with_live_progress(mut self, live_progress: bool) -> Self {
        self.live_progress = live_progress;
        self
    }

    pub(crate) fn take(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.outputs)
    }

    pub(crate) fn event(&mut self, event: &NormalizedTranscriptEvent) {
        match event.kind {
            NormalizedTranscriptKind::AssistantMessage => self.reply(event),
            NormalizedTranscriptKind::Plan => {
                self.planned = true;
                self.reply(event);
            }
            NormalizedTranscriptKind::UserMessage => {
                if self.message_id.is_none() {
                    self.message_id = event.event_id.strip_prefix("user:").map(str::to_owned);
                }
                self.user_messages += 1;
                if self.user_messages > 1 {
                    self.status(format!("› {}", first_line(&event.text)));
                }
            }
            NormalizedTranscriptKind::ToolActivity if !event.is_partial => {
                self.status(format!("  · {}", first_line(&event.text)));
            }
            NormalizedTranscriptKind::Notice => {
                self.last_notice = Some(event.text.trim().to_owned());
                self.status(format!("! {}", first_line(&event.text)));
            }
            NormalizedTranscriptKind::ToolActivity | NormalizedTranscriptKind::Thinking => {}
        }
    }

    fn reply(&mut self, event: &NormalizedTranscriptEvent) {
        self.close_progress();
        if event.is_partial {
            self.streamed.push_str(&event.text);
            self.reply_open = true;
            self.outputs.push(Output::Reply(event.text.clone()));
            return;
        }
        let text = match event.text.strip_prefix(self.streamed.as_str()) {
            Some(rest) if self.reply_open => rest.to_owned(),
            _ if self.reply_open => format!("\n{}", event.text),
            _ => event.text.clone(),
        };
        self.streamed.clear();
        self.reply_open = false;
        let newline = if text.ends_with('\n') { "" } else { "\n" };
        self.outputs.push(Output::Reply(format!("{text}{newline}")));
    }

    pub(crate) fn permission(&mut self, request: &PermissionDecisionRequest) {
        if !self.announced.insert(request.binding.decision_id.0.clone()) {
            return;
        }
        let respond = format!(
            "{} permissions respond --conversation-id {} --run-id {} --decision-id {} --decision",
            self.command_prefix,
            request.binding.conversation_id.0,
            request.binding.run_id.0,
            request.binding.decision_id.0,
        );
        let mut lines = vec![format!(
            "! Waiting for your permission to use {} ({:?})",
            request.request.tool_name, request.request.category
        )];
        if let Some(detail) = input_summary(request.request.input.as_ref()) {
            lines.push(format!("    {detail}"));
        }
        lines.push(format!("  Approve: {respond} approve-once"));
        lines.push(format!("  Deny:    {respond} deny"));
        self.status(lines.join("\n"));
    }

    pub(crate) fn provider_install(&mut self, provider: AgentChatProvider) {
        if !self.announced.insert("provider-install".into()) {
            return;
        }
        self.status(format!(
            "! Waiting for {} to be installed before this prompt can run.\n  Review the install: {} provider readiness --conversation-id {} --run-id {}",
            super::provider_name(provider),
            self.command_prefix,
            self.conversation_id,
            self.run_id,
        ));
    }

    pub(crate) fn install_hold(&mut self, hold: &crate::prompt_hold::InstallHold) {
        if !self
            .announced
            .insert(format!("provider-install:{}", hold.prompt_receipt_id))
        {
            return;
        }
        self.status(crate::prompt_hold::install_lines(hold, &self.command_prefix).join("\n"));
    }

    pub(crate) fn download(&mut self, model: &str, state: &LocalModelInstallState) {
        let LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        } = state
        else {
            return;
        };
        let percent = downloaded_bytes
            .saturating_mul(100)
            .checked_div(*total_bytes)
            .unwrap_or(0)
            .min(100);
        let line =
            crate::local_models_cli::progress_display(model, *downloaded_bytes, *total_bytes);
        if self.live_progress {
            self.close_reply();
            self.progress_open = true;
            self.outputs.push(Output::Status(format!("\r{line}")));
        } else if self
            .progress_percent
            .is_none_or(|previous| percent >= previous + 10 || percent == 100 && previous < 100)
        {
            self.progress_percent = Some(percent);
            self.status(line);
        }
    }

    pub(crate) fn finish(
        &mut self,
        phase: DurableTurnPhase,
        cause: Option<TurnTerminalCause>,
    ) -> Result<(), CliError> {
        self.close_progress();
        self.close_reply();
        if self.planned && phase == DurableTurnPhase::Completed {
            self.planned = false;
            self.status(format!(
                "Plan ready.\n  Approve: {prefix} plan start --conversation-id {conversation}\n  Reject:  {prefix} plan reject --conversation-id {conversation}",
                prefix = self.command_prefix,
                conversation = self.conversation_id,
            ));
        }
        if phase == DurableTurnPhase::Failed
            && cause == Some(TurnTerminalCause::ProviderSessionUnavailable)
        {
            if let Some(message_id) = self.message_id.clone() {
                self.status(format!(
                    "Continue from Gent's saved history: {}",
                    super::continuation::command(
                        &self.command_prefix,
                        &self.conversation_id,
                        &message_id
                    )
                ));
            }
        }
        outcome(phase, cause, self.last_notice.as_deref())
    }

    fn status(&mut self, text: String) {
        self.close_progress();
        self.close_reply();
        self.outputs.push(Output::Status(format!("{text}\n")));
    }

    fn close_reply(&mut self) {
        if self.reply_open {
            self.reply_open = false;
            self.streamed.clear();
            self.outputs.push(Output::Reply("\n".into()));
        }
    }

    fn close_progress(&mut self) {
        if self.progress_open {
            self.progress_open = false;
            self.outputs.push(Output::Status("\n".into()));
        }
    }
}

pub(crate) fn outcome(
    phase: DurableTurnPhase,
    cause: Option<TurnTerminalCause>,
    notice: Option<&str>,
) -> Result<(), CliError> {
    match (phase, cause) {
        (DurableTurnPhase::Completed, _)
        | (DurableTurnPhase::Interrupted, Some(TurnTerminalCause::Steered)) => Ok(()),
        (DurableTurnPhase::Failed, _) => Err(CliError::new(
            Failure::TurnFailed,
            notice.map_or_else(
                || "the turn failed without a reason from the provider".to_owned(),
                |notice| format!("the turn failed: {notice}"),
            ),
        )),
        (DurableTurnPhase::Cancelled, _) => Err(CliError::new(
            Failure::TurnInterrupted,
            "the prompt was cancelled before it finished",
        )),
        _ => Err(CliError::new(
            Failure::TurnInterrupted,
            "the turn was interrupted before it finished",
        )),
    }
}

fn first_line(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default();
    let mut clipped = line.chars().take(160).collect::<String>();
    if line.chars().count() > 160 || text.trim().lines().nth(1).is_some() {
        clipped.push('…');
    }
    clipped
}

fn input_summary(input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str))
    .map(first_line)
    .or_else(|| Some(first_line(&input.to_string())).filter(|text| text != "{}"))
}

#[cfg(test)]
#[path = "turn_view_tests.rs"]
mod tests;
