//! Production adapters deny exemptions after failed launch or untrusted source selection.
use super::internal_mcp_owned_run_support::*;
use crate::{
    claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptRunner, ClaudePromptStart},
    codex_prompt_lifecycle::CodexPromptExecution,
    permission_preflight::PermissionPreflight,
};
use gent_ports::PublicProviderRunner;
use gent_types::*;
use std::sync::{Arc, atomic::Ordering};

#[test]
fn claude_failed_forged_and_unselected_launches_do_not_get_internal_permissions() {
    for case in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let (path, mut servers) = config(root.path());
        if case == 1 {
            servers["gent-goal"]["command"] = serde_json::json!("external");
        }
        if case == 2 {
            servers = serde_json::json!({"external":{"command":"external","args":[],"env":{}}});
        }
        std::fs::write(&path, serde_json::json!({"mcpServers":servers}).to_string()).unwrap();
        let state = Arc::new(State::default());
        state.fail.store(case == 0, Ordering::SeqCst);
        let runner = ClaudePromptRunner::new(Launcher(state), buffer(), Some(path));
        runner
            .prepare(
                "run".into(),
                ClaudePromptStart {
                    conversation_id: "conversation".into(),
                    workspace_root: root.path().into(),
                    workspace_access: SandboxWorkspaceAccess::ReadWrite,
                    prompt: "test".into(),
                    turn_options:
                        gent_drivers::claude_turn_options::ClaudeTurnOptions::from_selection(
                            &selection(AgentChatProvider::Claude),
                        )
                        .unwrap(),
                    goal: None,
                    fresh_context: None,
                    content: vec![],
                    selected_mcp_source_names: if case == 2 {
                        vec!["external".into()]
                    } else {
                        vec![]
                    },
                    disallowed_mcp_tools: vec![],
                    recreate_session: false,
                },
            )
            .unwrap();
        let result = PublicProviderRunner::start(&runner, "run", &lock(root.path(), "claude"));
        assert_eq!(result.is_err(), case == 0);
        for tool in [
            "mcp__gent-goal__gent_goal_update",
            "mcp__gent_goal__gent_goal_update",
        ] {
            assert_eq!(
                runner.permission_preflight(
                    &preflight(Some(&injection(root.path()))),
                    &policy(),
                    &request(tool)
                ),
                PermissionPreflight::Ask,
                "case {case}"
            );
        }
    }
}

#[test]
fn codex_failed_forged_and_unselected_launches_do_not_get_internal_permissions() {
    use gent_drivers::codex_prompt_runner::{CodexPromptRunner, CodexPromptStart};
    for case in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let (path, servers) = config(root.path());
        let mut selected =
            crate::conversation_scoped_mcp::scope_server_map(&servers, "conversation");
        if case == 1 {
            selected["gent-goal"]["command"] = serde_json::json!("external");
        }
        if case == 2 {
            selected = serde_json::json!({"external":{"command":"external"}});
        }
        let state = Arc::new(State::default());
        state.fail.store(case == 0, Ordering::SeqCst);
        let runner = CodexPromptRunner::new(Launcher(state), buffer(), None, Some(path));
        runner
            .prepare_codex_prompt(
                "run".into(),
                "conversation".into(),
                CodexPromptStart {
                    working_directory: None,
                    workspace_root: root.path().into(),
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
                    mcp_servers: Some(selected),
                    interrupted_reply: None,
                },
            )
            .unwrap();
        let result = PublicProviderRunner::start(&runner, "run", &lock(root.path(), "codex"));
        assert_eq!(result.is_err(), case == 0);
        for tool in [
            "mcp__gent-goal__gent_goal_update",
            "mcp__gent_goal__gent_goal_update",
        ] {
            assert_eq!(
                runner.permission_preflight(
                    &preflight(Some(&injection(root.path()))),
                    &policy(),
                    &request(tool)
                ),
                PermissionPreflight::Ask,
                "case {case}"
            );
        }
    }
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
