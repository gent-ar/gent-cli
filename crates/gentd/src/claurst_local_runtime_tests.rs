use std::path::{Path, PathBuf};

use gent_testkit::host_absolute_path;

use super::{ClaurstLocalRuntimePlan, ClaurstLocalRuntimePlanError, ClaurstLocalRuntimeRequest};
use crate::local_model_catalog::LocalModelCatalog;

const MODEL: &str = "/opt/gent/models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf";
const TEMPLATE: &str = "/opt/gent/claurst/.claurst/templates/qwen3-tool-use.jinja";

fn request() -> ClaurstLocalRuntimeRequest {
    ClaurstLocalRuntimeRequest {
        claurst_executable: host_absolute_path("/opt/gent/bin/claurst"),
        llama_server_executable: host_absolute_path("/opt/gent/bin/llama-server"),
        model_path: host_absolute_path(MODEL),
        claurst_home: host_absolute_path("/opt/gent/claurst"),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Agent,
        permission_mode: gent_types::PermissionMode::AskEveryTime,
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

    assert_eq!(plan.model_path, host_absolute_path(MODEL));
    assert_eq!(plan.server_url, "http://127.0.0.1:18080");
    assert_eq!(
        plan.settings_path,
        host_absolute_path("/opt/gent/claurst/.claurst/settings.json")
    );
    let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
    assert_eq!(settings["provider"], "llama-cpp");
    assert_eq!(settings["config"]["provider"], "llama-cpp");
    assert_eq!(settings["config"]["model"], "llama-cpp/qwen3-8b-q4-k-m");
    assert_eq!(settings["config"]["mcp_servers"], serde_json::json!([]));
    assert_eq!(settings["config"]["permission_mode"], "default");
    assert_eq!(settings["config"]["max_tokens"], 4096);
    assert!(settings["config"]["custom_system_prompt"].is_null());
    assert!(
        kwargs(&plan)["gent_instructions"]
            .as_str()
            .unwrap()
            .contains("without a separate thinking phase")
    );
    assert!(kwargs(&plan).get("gent_tools").is_none());
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
            &host_absolute_path(MODEL).display().to_string(),
            "--host",
            "127.0.0.1",
            "--port",
            "18080",
            "--jinja",
            "--ctx-size",
            "32768",
            "--cache-type-k",
            "q8_0",
            "--cache-type-v",
            "q8_0",
            "--parallel",
            "1",
            "--reasoning",
            "off",
            "--reasoning-budget",
            "0",
            "--chat-template-file",
            &plan
                .chat_template_path
                .as_ref()
                .unwrap()
                .display()
                .to_string(),
            "--chat-template-kwargs",
            &kwargs(&plan).to_string(),
        ]
    );
    assert_eq!(plan.chat_template_path, Some(host_absolute_path(TEMPLATE)));
    assert!(
        plan.chat_template_contents
            .as_deref()
            .is_some_and(|template| template.contains("gent_tools"))
    );
    assert_eq!(plan.history_input_bytes, 58_344);
    assert_eq!(plan.claurst_acp.arguments, ["acp"]);
    assert_eq!(
        plan.claurst_acp.environment.get("LLAMA_CPP_HOST"),
        Some(&"http://127.0.0.1:18080".to_string())
    );
    assert_eq!(
        plan.claurst_acp.environment.get("HOME").map(Path::new),
        Some(host_absolute_path("/opt/gent/claurst").as_path())
    );
    assert_eq!(
        plan.claurst_acp
            .environment
            .get("CLAURST_HOME")
            .map(Path::new),
        Some(host_absolute_path("/opt/gent/claurst/.claurst").as_path())
    );
}

#[test]
fn projects_the_workspace_permission_posture_into_claurst_settings() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    for (permission_mode, expected) in [
        (gent_types::PermissionMode::AskEveryTime, "default"),
        (gent_types::PermissionMode::AutoAcceptEdits, "acceptEdits"),
        (gent_types::PermissionMode::Autonomous, "acceptEdits"),
        (gent_types::PermissionMode::Bypass, "bypassPermissions"),
    ] {
        let mut local_request = request();
        local_request.permission_mode = permission_mode;
        let plan = ClaurstLocalRuntimePlan::build(
            local_request,
            catalog.model("qwen3-8b-q4-k-m").unwrap(),
            18_080,
        )
        .unwrap();
        let settings: serde_json::Value = serde_json::from_str(&plan.settings_json).unwrap();
        assert_eq!(settings["config"]["permission_mode"], expected);
    }
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
    assert!(
        kwargs(&plan)["gent_instructions"]
            .as_str()
            .unwrap()
            .contains("concrete plan")
    );
    assert!(
        !kwargs(&plan)["gent_instructions"]
            .as_str()
            .unwrap()
            .contains("Glob")
    );
}

#[test]
fn compact_catalog_profile_exposes_only_core_tools_with_local_model_guidance() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let plan = ClaurstLocalRuntimePlan::build(
        request(),
        catalog.model("qwen3-1-7b-q4-k-m").unwrap(),
        18_080,
    )
    .unwrap();
    let kwargs = kwargs(&plan);
    assert_eq!(
        kwargs["gent_tools"],
        serde_json::json!(["Glob", "Grep", "Read", "Edit", "Write", "Bash"])
    );
    assert!(
        kwargs["gent_instructions"]
            .as_str()
            .unwrap()
            .contains("old_string must appear exactly once")
    );
    assert_eq!(plan.history_input_bytes, 75_744);
    let hermes = ClaurstLocalRuntimePlan::build(
        request(),
        catalog.model("hermes-3-llama-3-1-8b-q4-k-m").unwrap(),
        18_080,
    )
    .unwrap();
    assert!(
        !hermes
            .llama_server
            .arguments
            .contains(&"--chat-template-kwargs".to_owned())
    );
}

#[test]
fn compact_catalog_profile_offers_only_read_only_tools_outside_agent_mode() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    for mode in [
        gent_types::AgentChatMode::Ask,
        gent_types::AgentChatMode::Plan,
    ] {
        let mut request = request();
        request.mode = mode;
        let plan = ClaurstLocalRuntimePlan::build(
            request,
            catalog.model("qwen3-1-7b-q4-k-m").unwrap(),
            18_080,
        )
        .unwrap();
        assert_eq!(
            kwargs(&plan)["gent_tools"],
            serde_json::json!(["Glob", "Grep", "Read"]),
            "{mode:?}"
        );
    }
}

fn kwargs(plan: &ClaurstLocalRuntimePlan) -> serde_json::Value {
    let arguments = &plan.llama_server.arguments;
    let index = arguments
        .iter()
        .position(|argument| argument == "--chat-template-kwargs")
        .unwrap();
    serde_json::from_str(&arguments[index + 1]).unwrap()
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
        kwargs(&plan)["gent_instructions"]
            .as_str()
            .unwrap()
            .contains("always finish with a direct answer")
    );
    assert!(
        plan.llama_server
            .arguments
            .windows(2)
            .any(|arguments| { arguments == ["--reasoning".to_owned(), "on".to_owned()] })
    );
    assert!(
        plan.llama_server
            .arguments
            .windows(2)
            .any(|arguments| { arguments == ["--reasoning-effort".to_owned(), "high".to_owned()] })
    );
    assert!(
        plan.llama_server
            .arguments
            .windows(2)
            .any(|arguments| { arguments == ["--reasoning-budget".to_owned(), "1024".to_owned()] })
    );
}

#[test]
fn llama_context_size_comes_from_the_selected_catalog_entry() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    let mut model = catalog.model("qwen3-1-7b-q4-k-m").unwrap().clone();
    model.context_tokens = 16_384;
    let plan = ClaurstLocalRuntimePlan::build(request(), &model, 18_080).unwrap();
    assert!(
        plan.llama_server
            .arguments
            .windows(2)
            .any(|pair| pair == ["--ctx-size", "16384"])
    );
}
