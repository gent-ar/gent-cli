use super::internal_mcp_owned_run_support::*;
use crate::{
    claude_prompt_lifecycle::{ClaudePromptExecution, ClaudePromptRunner, ClaudePromptStart},
    permission_preflight::PermissionPreflight,
};
use gent_ports::PublicProviderRunner;
use gent_types::*;
use std::sync::{Arc, atomic::Ordering};
#[test]
fn claude_actual_preflight_requires_a_live_exact_injected_launch_and_accepts_early_callbacks() {
    let root = tempfile::tempdir().unwrap();
    let (path, _) = config(root.path());
    let state = Arc::new(State::default());
    let runner = ClaudePromptRunner::new(Launcher(state.clone()), buffer(), Some(path));
    let policy = policy();
    let name = "mcp__gent_goal__gent_goal_update";
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    runner
        .prepare(
            "run".into(),
            ClaudePromptStart {
                conversation_id: "conversation".into(),
                workspace_root: root.path().into(),
                workspace_access: SandboxWorkspaceAccess::ReadWrite,
                prompt: "test".into(),
                turn_options: gent_drivers::claude_turn_options::ClaudeTurnOptions::from_selection(
                    &selection(AgentChatProvider::Claude),
                )
                .unwrap(),
                goal: None,
                fresh_context: None,
                content: vec![],
                selected_mcp_source_names: vec![],
                disallowed_mcp_tools: vec![],
                recreate_session: false,
            },
        )
        .unwrap();
    // A requested/pending config carries no exemption.
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    PublicProviderRunner::start(&runner, "run", &lock(root.path(), "claude")).unwrap();
    let effects = runner.poll_claude_prompt("run").unwrap().unwrap();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        gent_drivers::claude_runner::ClaudeRunnerEffect::PermissionRequest(_)
    )));
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
        "mcp__gent-chat__gent_chat_list",
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
    for name in [
        "mcp__gent_goal__anything",
        "mcp__external__gent_goal_update",
    ] {
        assert_eq!(
            runner.permission_preflight(
                &preflight(Some(&injection(root.path()))),
                &policy,
                &request(name)
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
            &request("mcp__gent-chat__gent_chat_list")
        ),
        PermissionPreflight::Ask
    );
    assert_eq!(
        runner.permission_preflight(
            &crate::permission_preflight::PreflightContext {
                disallowed: &["mcp__gent_goal__*".into()],
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
    state.alive.store(false, Ordering::SeqCst);
    assert_eq!(
        runner.permission_preflight(
            &preflight(Some(&injection(root.path()))),
            &policy,
            &request(name)
        ),
        PermissionPreflight::Ask
    );
    runner.release_claude_session("run").unwrap();
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
