use std::path::PathBuf;

use super::{ClaurstLocalRuntimePlan, ClaurstLocalRuntimePlanError, ClaurstLocalRuntimeRequest};
use crate::local_model_catalog::LocalModelCatalog;

fn request() -> ClaurstLocalRuntimeRequest {
    ClaurstLocalRuntimeRequest {
        claurst_executable: PathBuf::from("/opt/gent/bin/claurst"),
        llama_server_executable: PathBuf::from("/opt/gent/bin/llama-server"),
        model_path: PathBuf::from("/opt/gent/models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf"),
        claurst_home: PathBuf::from("/opt/gent/claurst"),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::Default,
        mcp_servers: Vec::new(),
    }
}

#[test]
fn builds_an_isolated_llama_cpp_acp_plan_for_the_curated_model() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let plan = ClaurstLocalRuntimePlan::build(
        request(),
        catalog.model("qwen3-8b-q4-k-m").unwrap(),
        18_080,
    )
    .unwrap();

    assert_eq!(
        plan.model_path,
        PathBuf::from("/opt/gent/models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf",)
    );
    assert_eq!(plan.server_url, "http://127.0.0.1:18080");
    assert_eq!(
        plan.settings_path,
        PathBuf::from("/opt/gent/claurst/.claurst/settings.json")
    );
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert_eq!(settings["provider"], "llama-cpp");
    assert_eq!(settings["config"]["provider"], "llama-cpp");
    assert_eq!(settings["config"]["model"], "llama-cpp/qwen3-8b-q4-k-m");
    assert_eq!(settings["config"]["mcp_servers"], serde_json::json!([]));
    assert_eq!(settings["config"]["permission_mode"], "default");
    assert_eq!(settings["config"]["max_tokens"], 4096);
    assert!(
        settings["config"]["custom_system_prompt"]
            .as_str()
            .unwrap()
            .contains("/no_think")
    );
    assert_eq!(settings["config"]["enable_all_mcp_servers"], false);
    assert_eq!(
        settings["config"]["provider_configs"],
        serde_json::json!({})
    );
    assert_eq!(settings["providers"]["llama-cpp"], serde_json::json!({}));
    assert_eq!(
        plan.llama_server.arguments,
        vec![
            "-m",
            "/opt/gent/models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf",
            "--host",
            "127.0.0.1",
            "--port",
            "18080",
            "--jinja",
            "--ctx-size",
            "8192",
            "--parallel",
            "1",
        ]
    );
    assert_eq!(plan.chat_template_path, None);
    assert!(plan.chat_template_contents.is_none());
    assert_eq!(plan.claurst_acp.arguments, ["acp"]);
    assert_eq!(
        plan.claurst_acp.environment.get("LLAMA_CPP_HOST"),
        Some(&"http://127.0.0.1:18080".to_string())
    );
    assert_eq!(
        plan.claurst_acp.environment.get("HOME"),
        Some(&"/opt/gent/claurst".to_string())
    );
    assert_eq!(
        plan.claurst_acp.environment.get("CLAURST_HOME"),
        Some(&"/opt/gent/claurst/.claurst".to_string())
    );
}

#[test]
fn projects_the_workspace_permission_posture_into_claurst_settings() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut local_request = request();
    local_request.permission_mode = gent_types::PermissionMode::Bypass;
    let plan = ClaurstLocalRuntimePlan::build(
        local_request,
        catalog.model("qwen3-8b-q4-k-m").unwrap(),
        18_080,
    )
    .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert_eq!(settings["config"]["permission_mode"], "bypass-permissions");
}

#[test]
fn embeds_stdio_mcp_servers_in_claurst_settings() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut request = request();
    request.mcp_servers = vec![serde_json::json!({
        "name": "filesystem",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem"],
        "env": {"TOKEN": "secret"}
    })];
    let plan =
        ClaurstLocalRuntimePlan::build(request, catalog.model("qwen3-8b-q4-k-m").unwrap(), 18_080)
            .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert_eq!(
        settings["config"]["mcp_servers"],
        serde_json::json!([{
            "name": "filesystem",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem"],
            "env": {"TOKEN": "secret"}
        }])
    );
}

#[test]
fn rejects_relative_paths_and_invalid_port_before_any_effect() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut invalid = request();
    invalid.model_path = PathBuf::from("models");
    assert_eq!(
        ClaurstLocalRuntimePlan::build(invalid, catalog.models().first().unwrap(), 18_080),
        Err(ClaurstLocalRuntimePlanError::RelativePath("model file"))
    );
    let invalid = request();
    assert_eq!(
        ClaurstLocalRuntimePlan::build(invalid, catalog.models().first().unwrap(), 0),
        Err(ClaurstLocalRuntimePlanError::ZeroPort)
    );
}

#[test]
fn materializes_plan_mode_as_a_local_model_instruction() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut request = request();
    request.mode = gent_types::AgentChatMode::Plan;
    let plan =
        ClaurstLocalRuntimePlan::build(request, catalog.models().first().unwrap(), 18_080).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert!(
        settings["config"]["custom_system_prompt"]
            .as_str()
            .unwrap()
            .contains("plan")
    );
}

#[test]
fn maps_high_effort_to_qwen_thinking_with_a_bounded_response_budget() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut request = request();
    request.effort = gent_types::AgentChatEffort::High;
    let plan =
        ClaurstLocalRuntimePlan::build(request, catalog.models().first().unwrap(), 18_080).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert_eq!(settings["config"]["max_tokens"], 8192);
    assert!(
        settings["config"]["custom_system_prompt"]
            .as_str()
            .unwrap()
            .contains("/think")
    );
}
