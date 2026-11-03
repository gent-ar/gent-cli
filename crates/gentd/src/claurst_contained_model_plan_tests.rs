use super::*;
use gent_testkit::host_absolute_path;
#[test]
fn contained_model_configuration_preserves_catalog_effort_and_template_profile() {
    let catalog = crate::local_model_catalog::LocalModelCatalog::shipped().unwrap();
    let record = catalog.model("qwen3-8b-q4-k-m").unwrap();
    let plan = ClaurstLocalRuntimePlan::build(
        ClaurstLocalRuntimeRequest {
            claurst_executable: host_absolute_path("/gent/claurst.exe"),
            llama_server_executable: host_absolute_path("/gent/llama-server.exe"),
            model_path: host_absolute_path("/gent/model.gguf"),
            claurst_home: host_absolute_path("/gent/home"),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
            permission_mode: PermissionMode::Autonomous,
            mcp_servers: vec![],
        },
        record,
        65031,
    )
    .unwrap();
    assert_eq!(plan.model_record.id, record.id);
    assert_eq!(plan.model_effort, AgentChatEffort::Medium);
    assert_eq!(plan.model_mode, AgentChatMode::Agent);
    assert_eq!(plan.model_configuration["contextTokens"], 32768);
    assert_eq!(plan.model_configuration["maxOutputTokens"], 4096);
    assert_eq!(plan.model_configuration["reasoningBudget"], 0);
    assert_eq!(plan.model_configuration["enableThinking"], false);
    let at = plan
        .llama_server
        .arguments
        .iter()
        .position(|v| v == "--chat-template-kwargs")
        .unwrap();
    assert_eq!(
        plan.model_configuration["templateKwargs"],
        serde_json::from_str::<serde_json::Value>(&plan.llama_server.arguments[at + 1]).unwrap()
    );
    assert_eq!(plan.model_configuration["modelSha256"], record.sha256);
    assert_eq!(
        plan.model_configuration["modelId"],
        record.provider_model_id
    );
}
