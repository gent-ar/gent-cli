use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    claurst_local_runtime_owner::{SystemClaurstStandaloneLauncher, SystemPrivateSettingsStore},
    local_model_catalog::LocalModelCatalog,
    local_model_provisioning::LocalModelProvisioner,
};
use gent_ports::{ClaurstSourceId, ClaurstStartRequest, PrivateClaurstBridge};
use gent_types::{AgentChatConversationId, FrozenConversationContext};

use super::{
    ClaurstAcpStdio, ClaurstLocalReadinessService, ClaurstLocalRuntimeRequest,
    ClaurstStandaloneLauncher, ClaurstStandaloneOwner, ClaurstStandaloneStartError,
    LlamaServerReadiness, LocalProcessLaunch, LocalRuntimeProcess, PrivateSettingsStore,
};

#[derive(Clone, Default)]
struct Store(Arc<Mutex<Vec<String>>>);
impl PrivateSettingsStore for Store {
    fn materialize(&self, path: &Path, _: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push(format!("settings:{}", path.display()));
        Ok(())
    }
}

struct Llama(Arc<Mutex<Vec<String>>>);
impl LocalRuntimeProcess for Llama {
    fn exited(&mut self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn shutdown(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().push("shutdown".into());
        Ok(())
    }
}

struct Acp {
    events: Arc<Mutex<Vec<String>>>,
    reads: VecDeque<Vec<u8>>,
}
impl ClaurstAcpStdio for Acp {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push(format!("frame:{}", String::from_utf8_lossy(frame)));
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

#[derive(Clone)]
struct Launcher(Arc<Mutex<Vec<String>>>);
impl ClaurstStandaloneLauncher for Launcher {
    type Llama = Llama;
    type Acp = Acp;
    fn launch_llama(&self, _: &LocalProcessLaunch) -> Result<Llama, String> {
        self.0.lock().unwrap().push("llama".into());
        Ok(Llama(Arc::clone(&self.0)))
    }
    fn launch_acp(&self, _: &LocalProcessLaunch) -> Result<Acp, String> {
        self.0.lock().unwrap().push("acp".into());
        Ok(Acp {
            events: Arc::clone(&self.0),
            reads: VecDeque::from([
                serde_json::to_vec(&serde_json::json!({"id":1,"result":{}})).unwrap(),
                serde_json::to_vec(&serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}}))
                    .unwrap(),
            ]),
        })
    }
}

#[derive(Clone)]
struct Ready(Arc<Mutex<Vec<String>>>);
impl LlamaServerReadiness for Ready {
    fn wait_ready<P: LocalRuntimeProcess>(&self, _: &str, _: &mut P) -> Result<(), String> {
        self.0.lock().unwrap().push("ready".into());
        Ok(())
    }
}

fn request() -> ClaurstLocalRuntimeRequest {
    ClaurstLocalRuntimeRequest {
        claurst_executable: PathBuf::from("/bin/claurst"),
        llama_server_executable: PathBuf::from("/bin/llama-server"),
        model_path: PathBuf::from("/ignored"),
        claurst_home: PathBuf::from("/gent/claurst"),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::Default,
        mcp_servers: Vec::new(),
    }
}

fn owner(
    events: Arc<Mutex<Vec<String>>>,
    root: &Path,
) -> ClaurstStandaloneOwner<Store, Launcher, Ready> {
    let provisioner = LocalModelProvisioner::new(root, catalog());
    ClaurstStandaloneOwner::new(
        ClaurstLocalReadinessService::new(provisioner),
        Store(Arc::clone(&events)),
        Launcher(Arc::clone(&events)),
        Ready(events),
    )
}

fn catalog() -> LocalModelCatalog {
    LocalModelCatalog::from_json(
        r#"{"models":[{"id":"qwen2-5-coder-7b-instruct-q4-k-m","label":"Model","huggingface_url":"https://huggingface.co/gent/model/resolve/0123456789abcdef0123456789abcdef01234567/model.gguf","local_filename":"model.gguf","provider_model_id":"model","size_bytes":5,"sha256":"36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c"}]}"#,
    )
    .unwrap()
}

#[test]
fn missing_model_never_materializes_settings_or_starts_a_process() {
    let root = tempfile::tempdir().unwrap();
    let events = Arc::new(Mutex::new(vec![]));
    assert!(matches!(
        owner(Arc::clone(&events), root.path()).start(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            request(),
            Path::new("/workspace")
        ),
        Err(ClaurstStandaloneStartError::DownloadRequired {
            model_id,
            plan,
            downloaded_bytes: 0,
        }) if model_id == "qwen2-5-coder-7b-instruct-q4-k-m" && plan.model_id == model_id
    ));
    assert!(events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ready_model_starts_llama_then_acp_and_delivers_the_first_durable_prompt_to_acp() {
    let root = tempfile::tempdir().unwrap();
    let provisioner = LocalModelProvisioner::new(root.path(), catalog());
    let model = provisioner
        .plan("qwen2-5-coder-7b-instruct-q4-k-m")
        .unwrap();
    provisioner.ensure_storage(&model).unwrap();
    std::fs::write(&model.destination, b"abcde").unwrap();
    let events = Arc::new(Mutex::new(vec![]));
    let runtime = owner(Arc::clone(&events), root.path())
        .start(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            request(),
            Path::new("/workspace"),
        )
        .unwrap();
    let bridge = runtime.bridge();
    let binding = bridge
        .start(ClaurstStartRequest {
            run_id: "run-1".into(),
            source_id: ClaurstSourceId("source-1".into()),
            turn_id: "turn-1".into(),
            prompt: "hello".into(),
            context: FrozenConversationContext::cleared(AgentChatConversationId("c-1".into())),
            attachments: vec![],
            goal: None,
        })
        .await
        .unwrap();
    assert_eq!(binding.opaque_session_id, "acp-1");
    let events = events.lock().unwrap().clone();
    assert_eq!(
        &events[..4],
        [
            "settings:/gent/claurst/.claurst/settings.json",
            "llama",
            "ready",
            "acp"
        ]
    );
    assert!(
        events
            .iter()
            .any(|event| event.contains("\"method\":\"session/prompt\""))
    );
    drop(bridge);
    runtime.shutdown().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn system_owner_delivers_a_selected_prompt_through_real_acp_stdio() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let llama = root.path().join("fake-llama-server");
    let acp = root.path().join("fake-claurst");
    write_executable(
        &llama,
        "#!/bin/sh\ntrap 'exit 0' TERM INT\nwhile :; do sleep 1; done\n",
    );
    write_executable(
        &acp,
        r#"#!/bin/sh
IFS= read -r _
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
IFS= read -r _
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"fixture-session"}}'
IFS= read -r _
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-session","update":{"sessionUpdate":"agent_message_chunk","content":{"text":"fixture reply"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
while IFS= read -r _; do :; done
"#,
    );
    let provisioner = LocalModelProvisioner::new(root.path(), catalog());
    let model = provisioner
        .plan("qwen2-5-coder-7b-instruct-q4-k-m")
        .unwrap();
    provisioner.ensure_storage(&model).unwrap();
    std::fs::write(&model.destination, b"abcde").unwrap();
    let owner = ClaurstStandaloneOwner::new(
        ClaurstLocalReadinessService::new(provisioner),
        SystemPrivateSettingsStore,
        SystemClaurstStandaloneLauncher,
        Ready(Arc::new(Mutex::new(vec![]))),
    );
    let runtime = owner
        .start(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            ClaurstLocalRuntimeRequest {
                claurst_executable: acp,
                llama_server_executable: llama,
                model_path: model.destination,
                claurst_home: root.path().join("claurst-home"),
                effort: gent_types::AgentChatEffort::Medium,
                mode: gent_types::AgentChatMode::Agent,
                permission_mode: gent_types::PermissionMode::Default,
                mcp_servers: Vec::new(),
            },
            root.path(),
        )
        .unwrap();
    let bridge = runtime.bridge();
    let binding = bridge
        .start(ClaurstStartRequest {
            run_id: "run-system".into(),
            source_id: ClaurstSourceId("source-system".into()),
            turn_id: "turn-system".into(),
            prompt: "selected local conversation".into(),
            context: FrozenConversationContext::cleared(AgentChatConversationId("c-system".into())),
            attachments: vec![],
            goal: None,
        })
        .await
        .unwrap();
    assert_eq!(binding.opaque_session_id, "fixture-session");
    let mut facts = Vec::new();
    let mut after_cursor = 0;
    let mut terminal = None;
    for _ in 0..100 {
        let batch = bridge
            .drain(gent_ports::ClaurstDrainRequest {
                source_id: ClaurstSourceId("source-system".into()),
                run_id: "run-system".into(),
                after_cursor,
                limit: 16,
            })
            .await
            .unwrap();
        after_cursor += u64::try_from(batch.facts.len()).unwrap();
        facts.extend(batch.facts);
        terminal = batch.terminal;
        if terminal.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        matches!(
            facts.as_slice(),
            [
                partial,
                final_output,
            ] if matches!(&partial.value, gent_ports::ClaurstFactValue::Event(gent_types::NormalizedProviderEvent::Output { text, is_partial: true }) if text == "fixture reply")
                && matches!(&final_output.value, gent_ports::ClaurstFactValue::Event(gent_types::NormalizedProviderEvent::Output { text, is_partial: false }) if text == "fixture reply")
        ),
        "{facts:?}"
    );
    assert_eq!(terminal, Some(gent_ports::ClaurstTerminal::Completed));
    drop(bridge);
    runtime.shutdown().unwrap();

    fn write_executable(path: &Path, source: &str) {
        std::fs::write(path, source).unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}
