use super::internal_mcp_owned_run_support::*;
use crate::{
    codex_prompt_lifecycle::CodexPromptExecution, permission_preflight::PermissionPreflight,
};
use gent_ports::PublicProviderRunner;
use gent_types::*;
use std::sync::Arc;
#[test]
fn codex_actual_preflight_uses_started_selected_map_and_revokes_on_release_or_config_restart() {
    use gent_drivers::codex_prompt_runner::CodexPromptRunner;
    let root = tempfile::tempdir().unwrap();
    let (path, servers) = config(root.path());
    let state = Arc::new(State::default());
    let runner =
        CodexPromptRunner::new(Launcher(state.clone()), buffer(), None, Some(path.clone()));
    let policy = policy();
    let name = "mcp__gent_goal__gent_goal_update";
    let selected = crate::conversation_scoped_mcp::scope_server_map(&servers, "conversation");
    let prompt = codex_start(root.path(), &selected);
    runner
        .prepare_codex_prompt("run".into(), "conversation".into(), prompt.clone())
        .unwrap();
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    let lock = lock(root.path(), "codex");
    PublicProviderRunner::start(&runner, "run", &lock).unwrap();
    // No initialize/thread acknowledgment is needed for a callback buffered by the real owner.
    assert_eq!(
        runner.permission_preflight(&preflight(None), &policy, &request(name)),
        PermissionPreflight::Ask
    );
    let different =
        crate::standalone_mcp_config::InjectedInternalMcp::fixture(root.path(), "different-cli");
    assert_eq!(
        runner.permission_preflight(&preflight(Some(&different)), &policy, &request(name)),
        PermissionPreflight::Ask
    );
    for name in [
        name,
        "mcp__gent-goal__gent_goal_update",
        "mcp__gent_chat__gent_chat_list",
    ] {
        assert_eq!(
            runner.permission_preflight(
                &preflight(Some(&injection(root.path()))),
                &policy,
                &request(name)
            ),
            PermissionPreflight::Allow
        );
    }
    for unproven in [
        "mcp__gent_goal__anything",
        "mcp__external__gent_goal_update",
    ] {
        assert_eq!(
            runner.permission_preflight(
                &preflight(Some(&injection(root.path()))),
                &policy,
                &request(unproven)
            ),
            PermissionPreflight::Ask
        );
    }
    assert_eq!(
        runner.permission_preflight(
            &crate::permission_preflight::PreflightContext {
                conversation_id: "other",
                ..preflight(Some(&injection(root.path())))
            },
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    assert_eq!(
        runner.permission_preflight(
            &crate::permission_preflight::PreflightContext {
                run_id: "other",
                ..preflight(Some(&injection(root.path())))
            },
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    assert_eq!(
        runner.permission_preflight(
            &crate::permission_preflight::PreflightContext {
                conversation_id: "other",
                ..preflight(Some(&injection(root.path())))
            },
            &policy,
            &request("mcp__gent-chat__gent_chat_list")
        ),
        PermissionPreflight::Ask
    );
    assert_eq!(
        runner.permission_preflight(
            &crate::permission_preflight::PreflightContext {
                disallowed: &["mcp__gent-goal__*".into()],
                ..preflight(Some(&injection(root.path())))
            },
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
        let mut broad = policy.clone();
        broad.mode = mode;
        assert_eq!(
            runner.permission_preflight(
                &preflight(Some(&injection(root.path()))),
                &broad,
                &request(name)
            ),
            PermissionPreflight::Ask
        );
    }
    std::fs::write(&path, "{\"mcpServers\":{}}").unwrap();
    assert!(runner.refresh_mcp_config("run").unwrap());
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    runner
        .prepare_codex_prompt("run".into(), "conversation".into(), prompt)
        .unwrap();
    PublicProviderRunner::start(&runner, "run", &lock).unwrap();
    runner.release_session("run").unwrap();
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
}

fn preflight(
    injection: Option<&crate::standalone_mcp_config::InjectedInternalMcp>,
) -> crate::permission_preflight::PreflightContext<'_> {
    crate::permission_preflight::PreflightContext {
        run_id: "run",
        conversation_id: "conversation",
        disallowed: &[],
        injection,
        host_epoch: HostEpoch(1),
    }
}

fn codex_start(
    root: &std::path::Path,
    selected: &serde_json::Value,
) -> gent_drivers::codex_prompt_runner::CodexPromptStart {
    gent_drivers::codex_prompt_runner::CodexPromptStart {
        working_directory: None,
        workspace_root: root.into(),
        workspace_access: SandboxWorkspaceAccess::ReadWrite,
        prompt: "test".into(),
        goal: None,
        fresh_context: None,
        turn_options: gent_drivers::codex_session::CodexTurnOptions::from_selection(
            &selection(AgentChatProvider::Codex),
            None,
        )
        .unwrap(),
        attachments: vec![],
        mcp_servers: Some(selected.clone()),
        interrupted_reply: None,
    }
}
