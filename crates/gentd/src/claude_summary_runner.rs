use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::claude_prompt_lifecycle::ClaudeSummaryHook;
use gent_drivers::{
    PublicProvider, SystemLauncher,
    claude_turn_options::ClaudeTurnOptions,
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    launch_spec::{LaunchIntent, arguments},
    message_encoding::{PublicSession, encode_user_message},
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess},
};
use gent_ports::{ConversationSummaryRunner, PortError, PublicProviderResolver};
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_types::{Command, HostEpoch, ReceiptId, SandboxWorkspaceAccess};

const MAX_PROMPT_BYTES: usize = 24 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
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
            launcher: crate::node_runtime_lock::standalone_provider_launcher(MAX_OUTPUT_BYTES),
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
            || prompt.len() > MAX_PROMPT_BYTES
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
        let mut output = Vec::new();
        loop {
            while let Some(chunk) = process
                .next_stdout_chunk()
                .map_err(|error| PortError::Provider(error.to_string()))?
            {
                output.extend_from_slice(&chunk);
                if output.len() > MAX_OUTPUT_BYTES {
                    let _ = process.signal_tree(ProcessTreeSignal::Kill);
                    return Err(PortError::Unavailable(
                        "Claude summary output exceeded its bound".into(),
                    ));
                }
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
                return Err(PortError::Unavailable("Claude summary timed out".into()));
            }
            thread::sleep(Duration::from_millis(5));
        }
        while let Some(chunk) = process
            .next_stdout_chunk()
            .map_err(|error| PortError::Provider(error.to_string()))?
        {
            output.extend_from_slice(&chunk);
            if output.len() > MAX_OUTPUT_BYTES {
                return Err(PortError::Unavailable(
                    "Claude summary output exceeded its bound".into(),
                ));
            }
        }
        parse_result(&output)
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

fn parse_result(output: &[u8]) -> Result<String, PortError> {
    for line in output.split(|byte| *byte == b'\n') {
        let Ok(frame) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if frame.get("type").and_then(serde_json::Value::as_str) == Some("result") {
            if let Some(result) = frame
                .get("result")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return Ok(result.to_owned());
            }
        }
    }
    Err(PortError::Provider(
        "Claude summary returned no structured result".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_result;

    #[test]
    fn accepts_only_a_nonempty_structured_result() {
        assert_eq!(
            parse_result(br#"{"type":"result","result":"Title"}"#).unwrap(),
            "Title"
        );
        assert!(parse_result(br#"{"type":"assistant","result":"Title"}"#).is_err());
        assert!(parse_result(br#"{"type":"result","result":"  "}"#).is_err());
    }

    #[test]
    fn accepts_the_final_result_after_claude_streaming_frames() {
        let output = br#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"ignored"}]}}
{"type":"result","result":"```json\n{\"title\":\"Release checklist completed\",\"recap\":\"\"}\n```"}
"#;
        assert_eq!(
            parse_result(output).unwrap(),
            "```json\n{\"title\":\"Release checklist completed\",\"recap\":\"\"}\n```"
        );
    }
}
