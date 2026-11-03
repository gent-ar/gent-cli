use super::*;
use gent_ports::{AgentChatWorkspaceLedger, PolicyLedger};
use gent_types::*;
use std::path::PathBuf;
#[test]
#[ignore = "requires exact packaged model worker/helper and official Qwen fixture"]
fn actual_catalog_caller_model_readiness_summary_and_gateway() {
    let case = PathBuf::from(std::env::var_os("GENT_QA_MODEL_WORKING_ROOT").unwrap());
    std::fs::create_dir(&case).unwrap();
    let workspace = case.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let workspace = workspace.canonicalize().unwrap();
    let home = case.join("home");
    std::fs::create_dir(&home).unwrap();
    let ledger = SqliteLedger::in_memory().unwrap();
    let run = uuid::Uuid::new_v4().to_string();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("owned-create".into()),
                idempotency_key: "owned-create".into(),
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
    let record = crate::contained_model::native_tests::record();
    let sdk = PathBuf::from(std::env::var_os("GENT_QA_CLAURST_EXECUTABLE").unwrap());
    let plan = crate::claurst_local_runtime::ClaurstLocalRuntimePlan::build(
        crate::claurst_local_runtime::ClaurstLocalRuntimeRequest {
            claurst_executable: sdk.clone(),
            llama_server_executable: case.join("MUST-NOT-START-HOST-SERVER.exe"),
            model_path: PathBuf::from(std::env::var_os("GENT_QA_MODEL_FILE").unwrap())
                .canonicalize()
                .unwrap(),
            claurst_home: home,
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Agent,
            permission_mode: PermissionMode::Autonomous,
            mcp_servers: vec![],
        },
        &record,
        65031,
    )
    .unwrap();
    assert_eq!(plan.model_configuration["maxOutputTokens"], 2048);
    let binding =
        SandboxPermissionBinding::new("conversation".into(), run.clone(), policy, HostEpoch(1))
            .unwrap();
    let held = Mutex::new(None);
    assert!(host(&plan.llama_server, true).is_err());
    let mut wrong = plan.clone();
    wrong.model_record.id = "different-model".into();
    assert!(launch(&wrong, Some(&binding), Some(&ledger), &held).is_err());
    assert!(held.lock().unwrap().is_none());
    let mut process = launch(&plan, Some(&binding), Some(&ledger), &held).unwrap();
    let model = process.contained_model().unwrap();
    let root = model.test_root();
    crate::claurst_local_runtime_owner::LlamaServerReadiness::wait_ready(
        &crate::claurst_local_runtime_owner::HttpLlamaServerReadiness::default(),
        "http://127.0.0.1:1",
        &mut process,
    )
    .unwrap();
    let sdk = gent_drivers::lock::capture("claurst", &sdk, "1", "claurst-acp-v1").unwrap();
    let lease = crate::provider_egress::ProviderGatewayLease::claurst_contained(
        ledger,
        "conversation".into(),
        run,
        sdk,
        &case,
        model.clone(),
    )
    .unwrap();
    let summarizer =
        crate::claurst_runtime_factory::LlamaContextSummarizer::new(plan.summary_endpoint)
            .with_contained_model(Some(model.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let summary = runtime
        .block_on(
            crate::claurst_runtime_factory::ContextSummarizer::summarize(
                &summarizer,
                gent_drivers::conversation_context_summary::SummaryRequest {
                    instructions: "Return a short summary.",
                    content: "Owned test: one model actor with no network listener.".into(),
                },
            ),
        )
        .unwrap();
    assert!(!summary.text.is_empty() && summary.tokens > 0);
    drop(summarizer);
    drop(lease);
    process.shutdown().unwrap();
    assert!(model.settled());
    drop(process);
    drop(held);
    drop(model);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while root.exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!root.exists());
    assert!(!plan.llama_server.executable.exists());
    std::fs::write(std::env::var_os("GENT_QA_MODEL_REPORT").unwrap(),serde_json::to_vec_pretty(&serde_json::json!({
        "passed":true,"actualClaurstModelCaller":true,"catalogSelectionMismatchRejectedBeforeLaunch":true,"genericContainedHostFallbackRejected":true,
        "legacyHostServerImageAbsent":true,"nativeReadinessWithoutHttp":true,"actualProviderGatewayLease":true,"actualSummarySameActor":true,
        "knownRootRemoved":true,"providerSdkStarted":false
    })).unwrap()).unwrap();
}
