use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::claude_prompt_lifecycle::ClaudeSummaryHook;
use gent_drivers::{
    MAX_PROVIDER_FRAME_BYTES, PublicProvider, SystemLauncher,
    claude_turn_options::ClaudeTurnOptions,
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    launch_spec::{LaunchIntent, arguments},
    message_encoding::{PublicSession, encode_user_message},
    ndjson::NdjsonFramer,
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess},
};
use gent_ports::{ConversationSummaryRunner, PortError, PublicProviderResolver};
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_types::{Command, HostEpoch, ReceiptId, SandboxWorkspaceAccess};

const DIAGNOSTIC_CAPTURE_BYTES: usize = 16 * 1024;
const MAX_RUNTIME: Duration = crate::provider_launch_budget::launch_budget(Duration::from_secs(30));

#[derive(Clone)]
pub(crate) struct ClaudeSummaryRunner {
    launcher: SystemLauncher,
    resolver: Arc<dyn PublicProviderResolver>,
}

impl std::fmt::Debug for ClaudeSummaryRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClaudeSummaryRunner")
            .finish_non_exhaustive()
    }
}

impl ClaudeSummaryRunner {
    pub(crate) fn new(resolver: Arc<dyn PublicProviderResolver>) -> Self {
        Self {
            launcher: crate::node_runtime_lock::standalone_provider_launcher(
                DIAGNOSTIC_CAPTURE_BYTES,
            ),
            resolver,
        }
    }
}

impl ConversationSummaryRunner for ClaudeSummaryRunner {
    fn run_summary(
        &self,
        provider: &str,
        model_version: &str,
        prompt: &str,
    ) -> Result<String, PortError> {
        if provider != "claude"
            || model_version.trim().is_empty()
            || model_version.contains('\0')
            || prompt.trim().is_empty()
        {
            return Err(PortError::Unavailable(
                "Claude summary request is invalid".into(),
            ));
        }
        let mut launch_arguments = arguments("claude", &LaunchIntent::Start)
            .map_err(|error| PortError::Provider(error.to_string()))?;
        ClaudeTurnOptions::summary(model_version)
            .map_err(|error| PortError::Provider(error.to_string()))?
            .append_arguments(&mut launch_arguments);
        let lock = self
            .resolver
            .resolve("claude")
            .map_err(|error| PortError::Unavailable(error.to_string()))?;
        let launch = ProviderLaunch {
            permission_binding: None,
            run_id: None,
            executable: PathBuf::from(&lock.canonical_path),
            lock,
            provider: "claude".into(),
            arguments: launch_arguments,
            intent: LaunchIntent::Start,
            workspace_root: None,
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
        };
        let process = self
            .launcher
            .launch(&launch)
            .map_err(|error| PortError::Provider(error.to_string()))?;
        let command = Command {
            receipt_id: ReceiptId::new(),
            idempotency_key: ReceiptId::new().0,
            host_epoch: HostEpoch(0),
            kind: "userMessage".into(),
            payload: serde_json::json!({"prompt": prompt}),
        };
        let input = encode_user_message(
            PublicProvider::Claude,
            &PublicSession::ClaudeStream,
            &command,
        )
        .map_err(|error| PortError::Provider(error.to_string()))?;
        process
            .write_frame(&input)
            .map_err(|error| PortError::Provider(error.to_string()))?;
        process
            .close_stdin()
            .map_err(|error| PortError::Provider(error.to_string()))?;
        let deadline = Instant::now() + MAX_RUNTIME;
        let mut reply = ClaudeReply::new()?;
        loop {
            while let Some(chunk) = process
                .next_stdout_chunk()
                .map_err(|error| PortError::Provider(error.to_string()))?
            {
                reply.push(&chunk);
            }
            if process
                .try_exit_code()
                .map_err(|error| PortError::Provider(error.to_string()))?
                .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = process.signal_tree(ProcessTreeSignal::Kill);
                return Err(PortError::Provider(
                    "Claude did not finish its answer in time".into(),
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        while let Some(chunk) = process
            .next_stdout_chunk()
            .map_err(|error| PortError::Provider(error.to_string()))?
        {
            reply.push(&chunk);
        }
        reply.finish()
    }
}

struct ClaudeReply {
    frames: NdjsonFramer,
    result: Option<Result<String, PortError>>,
}

impl ClaudeReply {
    fn new() -> Result<Self, PortError> {
        Ok(Self {
            frames: NdjsonFramer::new(MAX_PROVIDER_FRAME_BYTES)
                .map_err(|error| PortError::Provider(error.to_string()))?,
            result: None,
        })
    }

    fn push(&mut self, chunk: &[u8]) {
        for frame in self.frames.push(chunk) {
            if let Some(result) = result_frame(&frame) {
                self.result = Some(result);
            }
        }
    }

    fn finish(self) -> Result<String, PortError> {
        self.result.unwrap_or_else(|| {
            Err(PortError::Provider(
                "Claude returned no structured result".into(),
            ))
        })
    }
}

#[derive(Debug)]
pub(crate) struct ClaudeSummarySchedulerHook {
    scheduler: ConversationSummaryScheduler<gent_store::SqliteLedger, ClaudeSummaryRunner>,
}

impl ClaudeSummarySchedulerHook {
    pub(crate) fn new(ledger: gent_store::SqliteLedger, runner: ClaudeSummaryRunner) -> Self {
        Self {
            scheduler: ConversationSummaryScheduler::new(ledger, runner),
        }
    }
}

impl ClaudeSummaryHook for ClaudeSummarySchedulerHook {
    fn schedule(&self, conversation_id: &str) -> Result<(), gent_runtime::RuntimeError> {
        self.scheduler.schedule(conversation_id).map(|_| ())
    }
}

fn result_frame(frame: &[u8]) -> Option<Result<String, PortError>> {
    let frame = serde_json::from_slice::<serde_json::Value>(frame).ok()?;
    if frame.get("type").and_then(serde_json::Value::as_str) != Some("result") {
        return None;
    }
    let text = frame
        .get("result")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Some(
        if frame.get("is_error").and_then(serde_json::Value::as_bool) == Some(true) {
            Err(PortError::Provider(format!(
                "Claude reported an error: {text}"
            )))
        } else if text.trim().is_empty() {
            Err(PortError::Provider(
                "Claude returned an empty answer".into(),
            ))
        } else {
            Ok(text.to_owned())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::ClaudeReply;

    fn reply(chunks: &[&[u8]]) -> Result<String, gent_ports::PortError> {
        let mut reply = ClaudeReply::new().unwrap();
        for chunk in chunks {
            reply.push(chunk);
        }
        reply.finish()
    }

    #[test]
    fn accepts_only_a_nonempty_successful_result() {
        assert_eq!(
            reply(&[b"{\"type\":\"result\",\"result\":\"Title\"}\n"]).unwrap(),
            "Title"
        );
        assert!(reply(&[b"{\"type\":\"assistant\",\"result\":\"Title\"}\n"]).is_err());
        assert!(reply(&[b"{\"type\":\"result\",\"result\":\"  \"}\n"]).is_err());
        assert!(
            reply(&[
                b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Prompt is too long\"}\n"
            ])
            .is_err()
        );
    }

    #[test]
    fn keeps_only_the_answer_of_a_stream_far_larger_than_the_answer() {
        let echoed = "x".repeat(64 * 1024);
        let answer = "y".repeat(48 * 1024);
        let stream = format!(
            "{{\"type\":\"system\",\"subtype\":\"init\"}}\n{}\n{}{}\n",
            serde_json::json!({"type": "user", "isReplay": true, "message": {"role": "user", "content": [{"type": "text", "text": echoed}]}}),
            "{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"y\"}}}\n"
                .repeat(2048),
            serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": answer}),
        );
        let chunks: Vec<&[u8]> = stream.as_bytes().chunks(4096).collect();
        assert_eq!(reply(&chunks).unwrap(), answer);
    }
}
