use super::{ClaurstLocalRuntimePlan, LocalModelCatalog, request};

#[test]
fn all_postures_keep_adapter_callbacks_for_gent_policy_and_chat_mode_vetoes() {
    let catalog = LocalModelCatalog::shipped().unwrap();
    for (permission_mode, expected) in [
        (gent_types::PermissionMode::AskEveryTime, "default"),
        (gent_types::PermissionMode::AutoAcceptEdits, "default"),
        (gent_types::PermissionMode::Autonomous, "default"),
        (gent_types::PermissionMode::Bypass, "default"),
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
