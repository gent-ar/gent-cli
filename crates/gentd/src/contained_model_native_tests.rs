use super::*;
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_types::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
struct Permit {
    root: Arc<Mutex<Option<PathBuf>>>,
    released: Arc<AtomicBool>,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.released.store(
            self.root
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|p| !p.exists()),
            Ordering::Release,
        );
    }
}
pub(crate) fn record() -> crate::local_model_catalog::LocalModelRecord {
    let json = serde_json::json!({"models":[{"id":"qa-qwen","label":"Owned model factory test","huggingface_url":"https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/13ac06624948e4d8589cb4b6f90100837342fcb8/Qwen3-0.6B-Q4_0.gguf","local_filename":"Qwen3-0.6B-Q4_0.gguf","provider_model_id":"gent-contained-test","size_bytes":428_970_080u64,"sha256":"da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4","context_tokens":4096,"runtime_memory_bytes":1536u64*1024*1024,"agent_profile":"compact"}]});
    crate::local_model_catalog::LocalModelCatalog::from_json(&json.to_string())
        .unwrap()
        .model("qa-qwen")
        .unwrap()
        .clone()
}
fn start(case: &Path) -> (ModelEndpoint, Arc<AtomicBool>) {
    let (endpoint, released, _, _, _) = gateway_fixture(case);
    (endpoint, released)
}
pub(crate) fn gateway_fixture(
    case: &Path,
) -> (
    ModelEndpoint,
    Arc<AtomicBool>,
    gent_store::SqliteLedger,
    RunVersionLock,
    String,
) {
    let workspace = case.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let temporary = case.join("model-scopes");
    std::fs::create_dir(&temporary).unwrap();
    let ledger = gent_store::SqliteLedger::in_memory().unwrap();
    let run = uuid::Uuid::new_v4().to_string();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId(run.clone()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qa-qwen".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: workspace.to_str().unwrap().into(),
            },
        )
        .unwrap();
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode: PermissionMode::Autonomous,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
    };
    ledger.save_policy(&policy).unwrap();
    let sdk = PathBuf::from(std::env::var_os("GENT_QA_CLAURST_EXECUTABLE").unwrap());
    let sdk = gent_drivers::lock::capture("claurst", &sdk, "0.1.7", "claurst-acp-v1").unwrap();
    let coordinator = gent_runtime::Coordinator::new(ledger.clone(), CapabilitySet::default());
    let fence = Arc::new(
        crate::run_fence::RunFence::capture(
            ledger.clone(),
            coordinator,
            "conversation".into(),
            run.clone(),
            sdk.clone(),
        )
        .unwrap(),
    );
    assert!(fence.current());
    let model = PathBuf::from(std::env::var_os("GENT_QA_MODEL_FILE").unwrap())
        .canonicalize()
        .unwrap();
    let rec = record();
    let selected = SelectedModel::capture(rec.clone(), &model).unwrap();
    let config = serde_json::json!({"schema":1,"modelPath":model.to_str().unwrap(),"modelSha256":rec.sha256,"modelId":rec.provider_model_id,"contextTokens":rec.context_tokens,"maxOutputTokens":64,"threads":1,"reasoningBudget":0,"enableThinking":false,"templateKwargs":{}});
    let root = Arc::new(Mutex::new(None));
    let released = Arc::new(AtomicBool::new(false));
    let permit = Permit {
        root: root.clone(),
        released: released.clone(),
    };
    let binding =
        SandboxPermissionBinding::new("conversation".into(), run.clone(), policy, HostEpoch(1))
            .unwrap();
    let endpoint = WindowsModelFactory::packaged()
        .unwrap()
        .launch(
            selected,
            ModelRun {
                binding,
                sdk_lock: sdk.clone(),
                current: Arc::new(move || fence.current()),
                lifetime: vec![Box::new(permit)],
                writable_roots: vec![workspace.clone()],
            },
            &temporary,
            config,
        )
        .unwrap();
    *root.lock().unwrap() = Some(endpoint.scope.root_path());
    assert!(
        !endpoint
            .scope
            .request
            .profile
            .readable_roots()
            .contains(&workspace)
    );
    assert!(endpoint.scope.request.profile.writable_roots().is_empty());
    assert_eq!(
        endpoint.scope.request.profile.network(),
        &SandboxNetworkPolicy::Disabled
    );
    (endpoint, released, ledger, sdk, run)
}
fn request(stream: bool) -> ModelRequest {
    ModelRequest{method:"POST".into(),path:"/v1/chat/completions".into(),body:serde_json::to_vec(&serde_json::json!({"model":"gent-contained-test","messages":[{"role":"user","content":"Say hello."}],"max_tokens":64,"temperature":0,"stream":stream,"chat_template_kwargs":{"enable_thinking":false}})).unwrap()}
}
fn body(http: &[u8]) -> Vec<u8> {
    let at = http.windows(4).position(|p| p == b"\r\n\r\n").unwrap() + 4;
    assert!(http.starts_with(b"HTTP/1.1 200 "));
    let mut input = &http[at..];
    let mut out = Vec::new();
    loop {
        let end = input.windows(2).position(|p| p == b"\r\n").unwrap();
        let n = usize::from_str_radix(std::str::from_utf8(&input[..end]).unwrap(), 16).unwrap();
        input = &input[end + 2..];
        if n == 0 {
            break;
        }
        out.extend(&input[..n]);
        assert_eq!(&input[n..n + 2], b"\r\n");
        input = &input[n + 2..];
    }
    out
}
#[test]
#[ignore = "requires exact staged packaged model/helper manifest and official Qwen file"]
fn actual_packaged_factory_json_stream_cancel_and_unknown() {
    struct Closed {
        bytes: Vec<u8>,
    }
    impl Write for Closed {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.bytes.windows(6).any(|v| v == b"data: ") {
                return Err(std::io::Error::other("owned peer closed"));
            }
            self.bytes.extend(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let root = PathBuf::from(std::env::var_os("GENT_QA_MODEL_WORKING_ROOT").unwrap());
    std::fs::create_dir(&root).unwrap();
    let positive = root.join("positive");
    std::fs::create_dir(&positive).unwrap();
    let (ep, released) = start(&positive);
    let model_root = ep.scope.root_path();
    let mut http = Vec::new();
    ep.respond(request(false), &mut http)
        .unwrap_or_else(|e| panic!("model response {:?}", e.error));
    let value: serde_json::Value = serde_json::from_slice(&body(&http)).unwrap();
    assert!(value["choices"][0]["message"]["content"].is_string());
    assert!(value["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    let mut http = Vec::new();
    ep.respond(request(true), &mut http)
        .unwrap_or_else(|e| panic!("stream {:?}", e.error));
    let stream = String::from_utf8(body(&http)).unwrap();
    assert!(stream.contains("data: [DONE]"));
    assert!(ep.revoke().is_ok() && ep.settled());
    drop(ep);
    let deadline = Instant::now() + Duration::from_secs(3);
    while model_root.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!model_root.exists() && released.load(Ordering::Acquire));
    let cancelled = root.join("cancelled");
    std::fs::create_dir(&cancelled).unwrap();
    let (ep, released) = start(&cancelled);
    let cancelled_root = ep.scope.root_path();
    let mut closed = Closed { bytes: vec![] };
    assert!(ep.respond(request(true), &mut closed).is_err());
    assert!(closed.bytes.windows(6).any(|v| v == b"data: "));
    assert!(ep.settled() && !ep.current());
    drop(ep);
    let deadline = Instant::now() + Duration::from_secs(3);
    while cancelled_root.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!cancelled_root.exists() && released.load(Ordering::Acquire));
    let unknown = root.join("unknown");
    std::fs::create_dir(&unknown).unwrap();
    let (ep, released) = start(&unknown);
    let unknown_root = ep.scope.root_path();
    assert!(
        ep.scope
            .process
            .signal_tree(gent_drivers::interrupt::ProcessTreeSignal::Kill)
            .is_err()
    );
    assert!(ep.revoke().is_err() && !ep.settled());
    drop(ep);
    std::thread::sleep(Duration::from_millis(100));
    assert!(unknown_root.exists() && !released.load(Ordering::Acquire));
    assert!(std::fs::remove_file(unknown_root.join("launch.json")).is_err());
    let report = serde_json::json!({"passed":true,"factory":"WindowsModelFactory::packaged","runFence":"actual unchanged production RunFence with private SQLite/Coodinator metadata","json":true,"sse":true,"noUserWorkspaceGrant":true,"networkDisabled":true,"knownRootRemoved":true,"cancelDuringSseKnownRootRemoved":true,"unknownRetainsRootConfigPinAndLifetime":true,"providerSdkStarted":false,"actualClaurstGatewayComposition":false});
    std::fs::write(
        std::env::var_os("GENT_QA_MODEL_REPORT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
