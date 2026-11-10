use crate::claude_prompt_lifecycle::ClaudePromptRunner;
use crate::claude_prompt_lifecycle_tests::{compatibility_for_lock, profile, prompt};
use gent_drivers::{
    buffering::BufferPolicy,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError},
};
use gent_ports::{
    AgentChatReadLedger, AgentChatWorkspaceLedger, ConversationLedger, PublicProviderResolver,
    PublicProviderRunError,
};
use gent_types::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s"}"#;
const DENIAL: &str = "Failed to authenticate: OAuth session expired and could not be refreshed";

#[derive(Default)]
struct Stdio {
    reply: Option<String>,
    stdout: VecDeque<Vec<u8>>,
}

#[derive(Clone, Default)]
struct Process(Arc<Mutex<Stdio>>);

impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}

impl ProviderProcess for Process {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        let mut state = self.0.lock().unwrap();
        if let Some(reply) = state.reply.take() {
            state
                .stdout
                .push_back(format!("{INIT}\n{reply}\n").into_bytes());
        }
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.0.lock().unwrap().stdout.pop_front())
    }
}

struct Launcher(Process);

impl ProcessLauncher for Launcher {
    type Process = Process;
    fn launch(&self, _: &ProviderLaunch) -> Result<Process, SupervisorError> {
        Ok(self.0.clone())
    }
}

struct Resolver(RunVersionLock);

impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        assert_eq!(provider, "claude");
        Ok(self.0.clone())
    }
}

fn failed_turn(
    assistant_error: Option<&str>,
) -> (Vec<NormalizedTranscriptEvent>, DurableTurnPhase) {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("claude");
    std::fs::write(&executable, "fake claude").unwrap();
    let lock = gent_drivers::lock::capture("claude", &executable, "2.1.0", "auth-fixture").unwrap();
    let compatibility = compatibility_for_lock(&lock);
    let mut assistant = serde_json::json!({
        "type": "assistant",
        "message": {"role": "assistant", "model": "<synthetic>", "content": [{"type": "text", "text": DENIAL}]},
        "session_id": "s",
        "uuid": "u",
    });
    if let Some(error) = assistant_error {
        assistant["error"] = error.into();
    }
    let result = serde_json::json!({
        "type": "result", "subtype": "success", "is_error": true, "result": DENIAL, "session_id": "s",
    });
    let process = Process::default();
    process.0.lock().unwrap().reply = Some(format!("{assistant}\n{result}"));
    let runner = ClaudePromptRunner::new(
        Launcher(process),
        BufferPolicy::new(16, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 0, 0).unwrap(),
        None,
    );
    let ledger = gent_store::SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("auth-conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("auth-create".into()),
                idempotency_key: "auth-create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("auth-run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "auth-workspace".into(),
                canonical_path: root.path().display().to_string(),
            },
        )
        .unwrap();
    let saved = prompt(&ledger, &conversation, "auth");
    let runtime = crate::public_driver_runtime::PublicDriversRuntime::new(
        profile(&compatibility),
        gent_runtime::Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner,
        Resolver(lock),
    )
    .unwrap();
    let mut host = crate::approved_claude_host::ApprovedClaudeHost::new(
        runtime,
        "auth-owner".into(),
        HostEpoch(1),
        1,
        None,
    );
    for _ in 0..16 {
        host.tick().unwrap();
    }
    let events = ledger
        .read_agent_chat_transcript(&conversation.0, None, 64)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event.turn_id == saved.message.turn_id)
        .collect();
    (events, ledger.list_run_turns("auth-run").unwrap()[0].phase)
}

fn notices(events: &[NormalizedTranscriptEvent]) -> Vec<(String, Option<AgentChatFailureCode>)> {
    events
        .iter()
        .filter(|event| event.kind == NormalizedTranscriptKind::Notice)
        .map(|event| (event.text.clone(), event.failure_code))
        .collect()
}

#[test]
fn an_expired_claude_sign_in_fails_the_turn_with_one_typed_notice() {
    let (events, phase) = failed_turn(Some("authentication_failed"));
    assert_eq!(phase, DurableTurnPhase::Failed);
    assert_eq!(
        notices(&events),
        vec![(
            "Claude sign-in expired. Sign in again to continue.".into(),
            Some(AgentChatFailureCode::ProviderAuthRequired)
        )]
    );
    assert!(events.iter().any(|event| {
        event.kind == NormalizedTranscriptKind::AssistantMessage && event.text == DENIAL
    }));
}

#[test]
fn an_untyped_claude_error_keeps_the_generic_notice() {
    let (events, phase) = failed_turn(None);
    assert_eq!(phase, DurableTurnPhase::Failed);
    assert_eq!(
        notices(&events),
        vec![("Claude ended the turn with an error.".into(), None)]
    );
}
