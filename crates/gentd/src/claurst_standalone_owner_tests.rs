use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
};

#[cfg(unix)]
use crate::claurst_local_runtime_owner::{
    SystemClaurstStandaloneLauncher, SystemPrivateSettingsStore,
};
use crate::{
    local_model_catalog::LocalModelCatalog, local_model_provisioning::LocalModelProvisioner,
};
use gent_ports::{ClaurstSourceId, ClaurstStartRequest, PrivateClaurstBridge};
use gent_testkit::host_absolute_path;
use gent_types::{AgentChatConversationId, FrozenConversationContext};

use super::{
    ClaurstAcpStdio, ClaurstLocalReadinessService, ClaurstLocalRuntimeRequest,
    ClaurstStandaloneLauncher, ClaurstStandaloneOwner, ClaurstStandaloneRuntime,
    ClaurstStandaloneStartError, LlamaServerReadiness, LocalProcessLaunch, LocalRuntimeProcess,
    PrivateSettingsStore,
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

#[test]
fn runtime_health_reports_an_exited_acp_before_reuse() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let bridge = Arc::new(crate::claurst_acp_bridge::ClaurstAcpBridge::new(
        host_absolute_path("/workspace"),
        Acp {
            events: Arc::clone(&events),
            reads: VecDeque::new(),
            exit: Some("ACP exited".into()),
        },
        Vec::new(),
    ));
    let mut runtime = ClaurstStandaloneRuntime {
        llama: Llama(events, 1),
        bridge,
        summarizer: Arc::new(crate::claurst_runtime_factory::LlamaContextSummarizer::new(
            crate::claurst_runtime_factory::LlamaSummaryEndpoint {
                server_url: "http://127.0.0.1:1".into(),
                model: "model".into(),
                context_tokens: 32_768,
                history_input_bytes: 81_888,
            },
        )),
        port: 1,
        llama_launch: LocalProcessLaunch {
            executable: host_absolute_path("/bin/llama-server"),
            arguments: Vec::new(),
            environment: std::collections::BTreeMap::new(),
            working_directory: None,
        },
    };
    assert_eq!(runtime.exited().unwrap(), Some("ACP exited".into()));
}

struct Llama(Arc<Mutex<Vec<String>>>, u64);
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
    exit: Option<String>,
}
impl Drop for Acp {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("acp-released".into());
    }
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
    fn exited(&mut self) -> Result<Option<String>, String> {
        Ok(self.exit.clone())
    }
}

#[derive(Clone)]
struct Launcher(Arc<Mutex<Vec<String>>>);
impl ClaurstStandaloneLauncher for Launcher {
    type Llama = Llama;
    type Acp = Acp;
    fn launch_llama(&self, _: &LocalProcessLaunch) -> Result<Llama, String> {
        let mut events = self.0.lock().unwrap();
        events.push("llama".into());
        let started = events.iter().filter(|event| *event == "llama").count() as u64;
        drop(events);
        Ok(Llama(Arc::clone(&self.0), started))
    }
    fn launch_acp(&self, launch: &LocalProcessLaunch) -> Result<Acp, String> {
        self.0
            .lock()
            .unwrap()
            .push(match &launch.working_directory {
                Some(directory) => format!("acp:{}", directory.display()),
                None => "acp".into(),
            });
        Ok(Acp {
            events: Arc::clone(&self.0),
            reads: VecDeque::from([
                serde_json::to_vec(&serde_json::json!({"id":1,"result":{}})).unwrap(),
                serde_json::to_vec(&serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}}))
                    .unwrap(),
            ]),
            exit: None,
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
        claurst_executable: host_absolute_path("/bin/claurst"),
        llama_server_executable: host_absolute_path("/bin/llama-server"),
        model_path: host_absolute_path("/ignored"),
        claurst_home: host_absolute_path("/gent/claurst"),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::AskEveryTime,
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
        r#"{"models":[{"id":"qwen2-5-coder-7b-instruct-q4-k-m","label":"Model","huggingface_url":"https://huggingface.co/gent/model/resolve/0123456789abcdef0123456789abcdef01234567/model.gguf","local_filename":"model.gguf","provider_model_id":"model","size_bytes":5,"sha256":"36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c","context_tokens":8192,"runtime_memory_bytes":4096,"agent_profile":"full"},{"id":"qwen3-templated","label":"Templated","huggingface_url":"https://huggingface.co/gent/model/resolve/0123456789abcdef0123456789abcdef01234567/model.gguf","local_filename":"templated.gguf","provider_model_id":"templated","size_bytes":5,"sha256":"36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c","context_tokens":8192,"runtime_memory_bytes":4096,"agent_profile":"full","chat_template_file":"qwen3-tool-use.jinja"}]}"#,
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
            &host_absolute_path("/workspace")
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
            &host_absolute_path("/workspace"),
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
            "llama".into(),
            "ready".into(),
            format!(
                "settings:{}",
                host_absolute_path("/gent/claurst")
                    .join(".claurst/settings.json")
                    .display()
            ),
            format!("acp:{}", host_absolute_path("/workspace").display()),
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
                permission_mode: gent_types::PermissionMode::AskEveryTime,
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
}

fn installed(root: &Path) -> LocalModelProvisioner {
    let provisioner = LocalModelProvisioner::new(root, catalog());
    let model = provisioner
        .plan("qwen2-5-coder-7b-instruct-q4-k-m")
        .unwrap();
    provisioner.ensure_storage(&model).unwrap();
    std::fs::write(&model.destination, b"abcde").unwrap();
    provisioner
}

fn chat_server(conversation_id: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "gent-chat",
        "command": "gent",
        "args": ["mcp", "chat", "--conversation-id", conversation_id],
        "env": [],
    })
}

#[test]
fn rebinding_a_session_keeps_the_same_llama_process_and_reloads_no_model() {
    let root = tempfile::tempdir().unwrap();
    let _ = installed(root.path());
    let events = Arc::new(Mutex::new(vec![]));
    let owner = owner(Arc::clone(&events), root.path());
    let mut first = request();
    first.mcp_servers = vec![chat_server("conversation-1")];
    let mut runtime = owner
        .start_with_mcp(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            first,
            &host_absolute_path("/workspace"),
            vec![chat_server("conversation-1")],
        )
        .unwrap();
    let llama_before = runtime.llama().1;
    let port = runtime.port();

    let mut second = request();
    second.mcp_servers = vec![chat_server("conversation-2")];
    owner
        .rebind_session(
            &mut runtime,
            "qwen2-5-coder-7b-instruct-q4-k-m",
            second,
            &host_absolute_path("/workspace"),
            vec![chat_server("conversation-2")],
        )
        .unwrap();

    assert_eq!(runtime.llama().1, llama_before);
    assert_eq!(runtime.port(), port);
    let events = events.lock().unwrap().clone();
    assert_eq!(
        events.iter().filter(|event| *event == "llama").count(),
        1,
        "llama.cpp must start exactly once across a conversation switch"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("acp:"))
            .count(),
        2,
        "only the Claurst ACP agent is replaced"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "acp-released")
            .count(),
        1,
        "the previous conversation's agent is released"
    );
    assert!(!events.iter().any(|event| event == "shutdown"));
}

#[test]
fn a_rebound_session_writes_only_the_new_conversations_chat_server() {
    let root = tempfile::tempdir().unwrap();
    let _ = installed(root.path());
    let events = Arc::new(Mutex::new(vec![]));
    let written = Arc::new(Mutex::new(Vec::new()));
    let owner = ClaurstStandaloneOwner::new(
        ClaurstLocalReadinessService::new(LocalModelProvisioner::new(root.path(), catalog())),
        Recorder(Arc::clone(&written)),
        Launcher(Arc::clone(&events)),
        Ready(Arc::clone(&events)),
    );
    let mut first = request();
    first.mcp_servers = vec![chat_server("conversation-1")];
    let mut runtime = owner
        .start_with_mcp(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            first,
            &host_absolute_path("/workspace"),
            Vec::new(),
        )
        .unwrap();
    let mut second = request();
    second.mcp_servers = vec![chat_server("conversation-2")];
    owner
        .rebind_session(
            &mut runtime,
            "qwen2-5-coder-7b-instruct-q4-k-m",
            second,
            &host_absolute_path("/workspace"),
            Vec::new(),
        )
        .unwrap();

    let settings = written.lock().unwrap().last().unwrap().clone();
    assert!(settings.contains("--conversation-id"));
    assert!(settings.contains("conversation-2"));
    assert!(!settings.contains("conversation-1"));
}

#[derive(Clone)]
struct Recorder(Arc<Mutex<Vec<String>>>);
impl PrivateSettingsStore for Recorder {
    fn materialize(&self, path: &Path, contents: &str) -> Result<(), String> {
        if path.ends_with("settings.json") {
            self.0.lock().unwrap().push(contents.to_owned());
        }
        Ok(())
    }
}

#[test]
fn a_changed_llama_plan_refuses_to_rebind_instead_of_reusing_the_wrong_model() {
    let root = tempfile::tempdir().unwrap();
    let _ = installed(root.path());
    let events = Arc::new(Mutex::new(vec![]));
    let owner = owner(Arc::clone(&events), root.path());
    let mut runtime = owner
        .start_with_mcp(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            request(),
            &host_absolute_path("/workspace"),
            Vec::new(),
        )
        .unwrap();
    let mut changed = request();
    changed.llama_server_executable = host_absolute_path("/bin/other-llama-server");
    assert!(matches!(
        owner.rebind_session(
            &mut runtime,
            "qwen2-5-coder-7b-instruct-q4-k-m",
            changed,
            &host_absolute_path("/workspace"),
            Vec::new(),
        ),
        Err(ClaurstStandaloneStartError::ModelPlanChanged)
    ));
}

#[cfg(unix)]
#[test]
fn a_real_conversation_switch_leaves_the_running_llama_process_untouched() {
    let root = tempfile::tempdir().unwrap();
    let llama = root.path().join("fake-llama-server");
    let acp = root.path().join("fake-claurst");
    write_executable(
        &llama,
        "#!/bin/sh\ntrap 'exit 0' TERM INT\nwhile :; do sleep 1; done\n",
    );
    write_executable(
        &acp,
        "#!/bin/sh\ntrap 'exit 0' TERM INT\nwhile :; do sleep 1; done\n",
    );
    let provisioner = installed(root.path());
    let model = provisioner
        .plan("qwen2-5-coder-7b-instruct-q4-k-m")
        .unwrap();
    let owner = ClaurstStandaloneOwner::new(
        ClaurstLocalReadinessService::new(LocalModelProvisioner::new(root.path(), catalog())),
        SystemPrivateSettingsStore,
        SystemClaurstStandaloneLauncher,
        Ready(Arc::new(Mutex::new(vec![]))),
    );
    let mut request = ClaurstLocalRuntimeRequest {
        claurst_executable: acp,
        llama_server_executable: llama,
        model_path: model.destination,
        claurst_home: root.path().join("claurst-home"),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::AskEveryTime,
        mcp_servers: vec![chat_server("conversation-1")],
    };
    let mut runtime = owner
        .start_with_mcp(
            "qwen2-5-coder-7b-instruct-q4-k-m",
            request.clone(),
            root.path(),
            Vec::new(),
        )
        .unwrap();
    let llama_pid = runtime.llama().process_id();
    assert!(alive(llama_pid));

    request.mcp_servers = vec![chat_server("conversation-2")];
    owner
        .rebind_session(
            &mut runtime,
            "qwen2-5-coder-7b-instruct-q4-k-m",
            request,
            root.path(),
            Vec::new(),
        )
        .unwrap();

    assert_eq!(runtime.llama().process_id(), llama_pid);
    assert!(alive(llama_pid), "the loaded model must still be resident");
    let settings =
        std::fs::read_to_string(root.path().join("claurst-home/.claurst/settings.json")).unwrap();
    assert!(settings.contains("conversation-2"));
    assert!(!settings.contains("conversation-1"));
    runtime.shutdown().unwrap();
    assert!(!alive(llama_pid));
}

#[cfg(unix)]
fn write_executable(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, source).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_templated_model_writes_its_chat_template_before_llama_reads_it() {
    let root = tempfile::tempdir().unwrap();
    let provisioner = LocalModelProvisioner::new(root.path(), catalog());
    let model = provisioner.plan("qwen3-templated").unwrap();
    provisioner.ensure_storage(&model).unwrap();
    std::fs::write(&model.destination, b"abcde").unwrap();
    let events = Arc::new(Mutex::new(vec![]));
    let runtime = owner(Arc::clone(&events), root.path())
        .start_with_mcp(
            "qwen3-templated",
            request(),
            &host_absolute_path("/workspace"),
            Vec::new(),
        )
        .unwrap();

    let events = events.lock().unwrap().clone();
    let template = events
        .iter()
        .position(|event| event.contains("qwen3-tool-use.jinja"))
        .expect("the chat template is materialized");
    let llama = events.iter().position(|event| event == "llama").unwrap();
    let settings = events
        .iter()
        .position(|event| event.contains("settings.json"))
        .unwrap();
    assert!(
        template < llama,
        "llama.cpp reads --chat-template-file at launch"
    );
    assert!(llama < settings, "settings.json is read by the ACP agent");
    drop(runtime);
}
