use crate::{
    approved_claude_host::ApprovedClaudeHost,
    claude_authority_supervisor::{PrivateClaudeShutdown, PrivateClaudeSupervisor},
    claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptRunner},
    claude_prompt_lifecycle_tests::{profile, prompt},
    public_driver_runtime::PublicDriversRuntime,
};
use gent_drivers::{
    buffering::BufferPolicy,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    process::{SystemLauncher, SystemProcess},
    supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError},
};
use gent_ports::{
    AgentChatWorkspaceLedger, ConversationLedger, PublicProviderResolver, PublicProviderRunError,
};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CapabilitySet, DurableTurnPhase,
    HostEpoch, ReceiptId, RunVersionLock, WorkspaceRecord,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
struct Launcher {
    directory: PathBuf,
    launches: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct Process {
    inner: SystemProcess,
    directory: PathBuf,
}

impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        fs::write(self.directory.join("stop-requested"), b"requested")
            .map_err(|e| ProcessTreeError::Failed(e.to_string()))?;
        self.inner.write_frame(b"{\"type\":\"qaStop\"}\n")
    }
}

impl ProviderProcess for Process {
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        self.inner.write_frame(frame)
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.inner.close_stdin()
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.inner.next_stdout_chunk()
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        self.inner.try_exit_code()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = fs::write(self.directory.join("exit-allowed"), b"cleanup");
        let _ = self.inner.close_stdin();
    }
}

impl ProcessLauncher for Launcher {
    type Process = Process;
    fn launch(&self, request: &ProviderLaunch) -> Result<Process, SupervisorError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        let root = self.directory.display().to_string().replace('\'', "''");
        let script = SCRIPT.replace("OWNED_ROOT", &root);
        let mut launch = request.clone();
        launch.arguments = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script,
        ];
        let inner = SystemLauncher::new(65536).launch(&launch)?;
        Ok(Process {
            inner,
            directory: self.directory.clone(),
        })
    }
}

const SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$root='OWNED_ROOT'; $turn=0
while ($null -ne ($line=[Console]::ReadLine())) {
 $f=$line | ConvertFrom-Json
 if ($f.type -eq 'qaStop') {
  while (-not [IO.File]::Exists([IO.Path]::Combine($root,'exit-allowed'))) { Start-Sleep -Milliseconds 10 }
  exit 0
 }
 if ($f.type -eq 'user') {
  $turn++
  while (-not [IO.File]::Exists([IO.Path]::Combine($root,('reply-'+$turn)))) {
   if ([IO.File]::Exists([IO.Path]::Combine($root,'exit-allowed'))) { exit 0 }
   Start-Sleep -Milliseconds 10
  }
  [IO.File]::WriteAllText([IO.Path]::Combine($root,'observed.json'),(@{pid=$PID;turn=$turn}|ConvertTo-Json -Compress))
  if ($turn -eq 1) { [Console]::WriteLine('{"type":"system","subtype":"init","session_id":"owned-session"}') }
  [Console]::WriteLine('{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"owned-output"}]}}')
  [Console]::WriteLine('{"type":"result","subtype":"success","is_error":false,"session_id":"owned-session","result":"owned-output"}')
  [Console]::Out.Flush()
 }
}
"#;

#[derive(Clone)]
struct Resolver(RunVersionLock);
impl PublicProviderResolver for Resolver {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        assert_eq!(provider, "claude");
        Ok(self.0.clone())
    }
}

#[test]
fn actual_owned_stream_survives_terminal_follow_up_and_shutdown_waits_for_real_exit() {
    let directory = tempfile::tempdir().unwrap();
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let lock = gent_drivers::lock::capture("claude", &executable, "owned-fixture", "owned-fixture")
        .unwrap();
    let compatibility = crate::claude_prompt_lifecycle_tests::compatibility_for_lock(&lock);
    let launches = Arc::new(AtomicUsize::new(0));
    let runner = ClaudePromptRunner::new(
        Launcher {
            directory: directory.path().into(),
            launches: launches.clone(),
        },
        BufferPolicy::new(128, gent_drivers::MAX_PROVIDER_FRAME_BYTES, 32, 16384).unwrap(),
        None,
    );
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("owned-conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("owned-create".into()),
                idempotency_key: "owned-create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run-a".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "owned-fixture".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "owned-workspace".into(),
                canonical_path: directory.path().display().to_string(),
            },
        )
        .unwrap();
    let runtime = PublicDriversRuntime::new(
        profile(&compatibility),
        Coordinator::new(ledger.clone(), CapabilitySet::default()),
        ledger.clone(),
        compatibility,
        runner.clone(),
        Resolver(lock),
    )
    .unwrap();
    let host = ApprovedClaudeHost::new(runtime, "daemon-a".into(), HostEpoch(1), 1, None);
    let mut supervisor = PrivateClaudeSupervisor::new(host);
    let mut pid = None;
    for (index, key) in ["first", "follow-up"].into_iter().enumerate() {
        prompt(&ledger, &conversation, key);
        supervisor.wake().unwrap();
        assert!(supervisor.needs_drive());
        fs::write(
            directory.path().join(format!("reply-{}", index + 1)),
            b"reply",
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            supervisor.wake().unwrap();
            let turns = ledger.list_run_turns("run-a").unwrap();
            if turns.len() == index + 1
                && turns.last().unwrap().phase == DurableTurnPhase::Completed
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned peer did not complete turn: {turns:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!supervisor.needs_drive());
        assert!(runner.has_claude_session("run-a"));
        assert_eq!(launches.load(Ordering::SeqCst), 1);
        let observed: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("observed.json")).unwrap())
                .unwrap();
        assert_eq!(observed["turn"], index + 1);
        let current = observed["pid"].as_u64().unwrap();
        assert_eq!(*pid.get_or_insert(current), current);
        assert!(!directory.path().join("stop-requested").exists());
    }
    assert!(matches!(
        supervisor.request_shutdown().unwrap(),
        PrivateClaudeShutdown::Draining { active_runs: 1, .. }
    ));
    assert!(directory.path().join("stop-requested").exists());
    supervisor.wake().unwrap();
    assert!(supervisor.needs_drive());
    assert!(!supervisor.shutdown_complete());
    assert!(runner.has_claude_session("run-a"));
    fs::write(directory.path().join("exit-allowed"), b"normal-owned-exit").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !supervisor.shutdown_complete() {
        supervisor.wake().unwrap();
        assert!(
            Instant::now() < deadline,
            "owned process exit was not observed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!runner.has_claude_session("run-a"));
    assert!(!supervisor.needs_drive());
    assert!(
        ledger
            .list_run_turns("run-a")
            .unwrap()
            .iter()
            .all(|t| t.phase == DurableTurnPhase::Completed)
    );
}
