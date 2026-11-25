use std::path::{Path, PathBuf};

use gent_types::{
    AgentChatMode, PermissionCategory, PermissionDenialReason, PermissionMode, PermissionRequest,
    PolicyRecord, PolicyScope,
};
use serde_json::json;

use super::{ClaurstPermissionDecision, decide};

const ALLOW: ClaurstPermissionDecision = ClaurstPermissionDecision::Allow;
const ASK: ClaurstPermissionDecision = ClaurstPermissionDecision::Ask;
const PLAN_EDIT: ClaurstPermissionDecision =
    ClaurstPermissionDecision::Deny(Some(PermissionDenialReason::PlanModeEdit));

fn workspace() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("project/src")).unwrap();
    std::fs::create_dir_all(root.path().join("outside")).unwrap();
    let project = root.path().join("project");
    (root, project)
}

fn policy(mode: PermissionMode) -> PolicyRecord {
    PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: "workspace".into(),
        scope: PolicyScope::ProviderPermissions,
        revision: 1,
        mode,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: vec![],
        allowed_categories: vec![],
        allowed_commands: Vec::new(),
    }
}

fn request(
    tool: &str,
    category: PermissionCategory,
    input: serde_json::Value,
) -> PermissionRequest {
    PermissionRequest::new(tool.into(), category, Some(input), None)
}

#[test]
fn native_chat_delegation_is_a_preapproved_control_bound_to_run_containment() {
    use gent_types::{Containment, SandboxEnforcement};
    let (_root, project) = workspace();
    let chat = request(
        "mcp__gent-chat__gent_chat_create",
        PermissionCategory::Provider,
        json!({"label":"Child"}),
    );
    for (mode, containment, evidence, owned, expected) in [
        (
            PermissionMode::Autonomous,
            Containment::Unsandboxed,
            SandboxEnforcement::Unavailable,
            true,
            ALLOW,
        ),
        (
            PermissionMode::Autonomous,
            Containment::Unsandboxed,
            SandboxEnforcement::Unavailable,
            false,
            ASK,
        ),
        (
            PermissionMode::Autonomous,
            Containment::Sandboxed,
            SandboxEnforcement::Unavailable,
            true,
            ASK,
        ),
        (
            PermissionMode::Autonomous,
            Containment::Sandboxed,
            SandboxEnforcement::Enforced,
            true,
            ALLOW,
        ),
        (
            PermissionMode::AskEveryTime,
            Containment::Unsandboxed,
            SandboxEnforcement::Unavailable,
            true,
            ALLOW,
        ),
        (
            PermissionMode::AskEveryTime,
            Containment::Unsandboxed,
            SandboxEnforcement::Unavailable,
            false,
            ASK,
        ),
        (
            PermissionMode::Bypass,
            Containment::Unsandboxed,
            SandboxEnforcement::Unavailable,
            true,
            ALLOW,
        ),
        (
            PermissionMode::Bypass,
            Containment::Sandboxed,
            SandboxEnforcement::Unavailable,
            true,
            ASK,
        ),
        (
            PermissionMode::Bypass,
            Containment::Sandboxed,
            SandboxEnforcement::Enforced,
            true,
            ALLOW,
        ),
    ] {
        assert_eq!(
            super::decide_with_enforcement(
                AgentChatMode::Agent,
                &policy(mode).for_conversation(containment),
                &chat,
                &project,
                evidence,
                owned,
            ),
            expected,
            "{mode:?} {containment:?} {evidence:?} owned={owned}"
        );
    }
}

#[test]
fn session_internal_tools_are_allowed_in_every_mode_without_a_sandbox() {
    use gent_types::{Containment, SandboxEnforcement};
    let (_root, project) = workspace();
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        for containment in [Containment::Sandboxed, Containment::Unsandboxed] {
            for evidence in [
                SandboxEnforcement::Unavailable,
                SandboxEnforcement::Enforced,
            ] {
                for chat_mode in [
                    AgentChatMode::Agent,
                    AgentChatMode::Ask,
                    AgentChatMode::Plan,
                ] {
                    for tool in ["Agent", "Task", "ScheduleWakeup", "TodoWrite"] {
                        assert_eq!(
                            super::decide_with_enforcement(
                                chat_mode,
                                &policy(mode).for_conversation(containment),
                                &request(tool, PermissionCategory::Provider, json!({})),
                                &project,
                                evidence,
                                false,
                            ),
                            ALLOW,
                            "{tool} {chat_mode:?} {mode:?} {containment:?} {evidence:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_subagent_launch_exemption_never_matches_an_mcp_alias() {
    let (_root, project) = workspace();
    let alias = request(
        "mcp__gent-chat__Agent",
        PermissionCategory::Provider,
        json!({}),
    );
    assert_eq!(
        super::decide_with_enforcement(
            AgentChatMode::Agent,
            &policy(PermissionMode::Autonomous)
                .for_conversation(gent_types::Containment::Unsandboxed),
            &alias,
            &project,
            gent_types::SandboxEnforcement::Unavailable,
            false,
        ),
        ASK
    );
}

#[test]
fn questions_require_an_answer_in_every_mode_even_with_grants_and_sandbox() {
    let (_root, project) = workspace();
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        let mut current = policy(mode);
        current.allowed_tools = vec!["AskUserQuestion".into()];
        current.allowed_categories = vec![PermissionCategory::Provider];
        for name in ["AskUserQuestion", "ask_user"] {
            let question = request(name, PermissionCategory::Provider, json!({"questions": []}));
            assert_eq!(
                super::decide_with_enforcement(
                    AgentChatMode::Agent,
                    &current,
                    &question,
                    &project,
                    gent_types::SandboxEnforcement::Enforced,
                    false,
                ),
                ASK
            );
        }
    }
}

fn read(path: &str) -> PermissionRequest {
    request(
        "Read",
        PermissionCategory::Read,
        json!({ "file_path": path }),
    )
}

fn edit(path: &str) -> PermissionRequest {
    request(
        "Edit",
        PermissionCategory::Edit,
        json!({ "file_path": path }),
    )
}

fn bash(command: &str) -> PermissionRequest {
    request(
        "Bash",
        PermissionCategory::Command,
        json!({ "command": command }),
    )
}

fn agent(
    mode: PermissionMode,
    request: &PermissionRequest,
    workspace: &Path,
) -> ClaurstPermissionDecision {
    decide(AgentChatMode::Agent, &policy(mode), request, workspace)
}

#[test]
fn plan_refuses_edits_and_ask_allows_only_the_changes_the_user_granted() {
    let (_root, project) = workspace();
    let mut granted = policy(PermissionMode::Bypass);
    granted.allowed_categories = vec![PermissionCategory::Command, PermissionCategory::Edit];
    granted.allowed_tools = vec!["Bash".into(), "Edit".into()];
    for (mode, change, command) in [
        (AgentChatMode::Ask, ALLOW, ALLOW),
        (
            AgentChatMode::Plan,
            PLAN_EDIT,
            decide(AgentChatMode::Agent, &granted, &bash("ls"), &project),
        ),
    ] {
        assert_eq!(
            decide(mode, &granted, &edit("src/app.py"), &project),
            change
        );
        assert_eq!(decide(mode, &granted, &bash("ls"), &project), command);
        assert_eq!(
            decide(
                mode,
                &policy(PermissionMode::AskEveryTime),
                &read("src/app.py"),
                &project
            ),
            ALLOW
        );
    }
    for ungranted in [PermissionMode::AskEveryTime, PermissionMode::Bypass] {
        assert_eq!(
            decide(
                AgentChatMode::Ask,
                &policy(ungranted),
                &edit("src/app.py"),
                &project
            ),
            ASK
        );
        assert_eq!(
            decide(
                AgentChatMode::Ask,
                &policy(ungranted),
                &bash("ls"),
                &project
            ),
            ASK
        );
    }
}

#[test]
fn plan_commands_follow_the_conversation_permission_mode() {
    let (_root, project) = workspace();
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        for containment in [
            gent_types::Containment::Sandboxed,
            gent_types::Containment::Unsandboxed,
        ] {
            let current = policy(mode).for_conversation(containment);
            for command in ["git --version", "py -3 --version"] {
                assert_eq!(
                    decide(AgentChatMode::Plan, &current, &bash(command), &project),
                    decide(AgentChatMode::Agent, &current, &bash(command), &project),
                    "{mode:?} {containment:?}"
                );
            }
            assert_eq!(
                decide(AgentChatMode::Plan, &current, &edit("src/app.py"), &project),
                PLAN_EDIT
            );
        }
        assert_eq!(
            decide(
                AgentChatMode::Plan,
                &policy(PermissionMode::AskEveryTime),
                &bash("git --version"),
                &project
            ),
            ASK
        );
    }
}

#[test]
fn unsandboxed_out_of_workspace_reads_are_declined_not_prompted() {
    let (root, project) = workspace();
    let outside = root.path().join("outside/secret.txt");
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
    ] {
        let policy = policy(mode).for_conversation(gent_types::Containment::Unsandboxed);
        assert_eq!(
            super::decide_with_enforcement(
                AgentChatMode::Agent,
                &policy,
                &read(outside.to_str().unwrap()),
                &project,
                gent_types::SandboxEnforcement::Unavailable,
                false,
            ),
            ClaurstPermissionDecision::Deny(None),
            "{mode:?}"
        );
    }
}

#[test]
fn workspace_reads_are_automatic_only_under_a_supported_policy() {
    let (root, project) = workspace();
    let outside = root.path().join("outside/secret.txt");
    let glob = request(
        "Glob",
        PermissionCategory::Read,
        json!({ "pattern": "**/*" }),
    );
    let absolute_glob = request(
        "Glob",
        PermissionCategory::Read,
        json!({ "pattern": "/etc/**" }),
    );
    assert_eq!(agent(PermissionMode::AskEveryTime, &glob, &project), ALLOW);
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &read("src/app.py"), &project),
        ALLOW
    );
    assert_eq!(
        agent(
            PermissionMode::Autonomous,
            &read("../outside/secret.txt"),
            &project
        ),
        ASK
    );
    assert_eq!(
        agent(
            PermissionMode::Autonomous,
            &read(outside.to_str().unwrap()),
            &project
        ),
        ASK
    );
    assert_eq!(
        agent(PermissionMode::Autonomous, &absolute_glob, &project),
        ASK
    );
    assert_eq!(agent(PermissionMode::Bypass, &absolute_glob, &project), ASK);
}

#[cfg(unix)]
#[test]
fn edits_through_a_symlink_that_leaves_the_workspace_follow_the_mode_matrix() {
    let (root, project) = workspace();
    std::os::unix::fs::symlink(root.path().join("outside"), project.join("escape")).unwrap();
    assert_eq!(
        agent(
            PermissionMode::AutoAcceptEdits,
            &edit("escape/x.py"),
            &project
        ),
        ClaurstPermissionDecision::Deny(None)
    );
    for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
        assert_eq!(agent(mode, &edit("escape/x.py"), &project), ASK);
    }
}

#[test]
fn edits_are_unattended_only_inside_the_workspace_in_an_edit_accepting_mode() {
    let (_root, project) = workspace();
    let inside = project.join("src/new.py");
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &edit("src/app.py"), &project),
        ASK
    );
    {
        let mode = PermissionMode::AutoAcceptEdits;
        assert_eq!(agent(mode, &edit("src/app.py"), &project), ALLOW);
        assert_eq!(
            agent(mode, &edit(inside.to_str().unwrap()), &project),
            ALLOW
        );
        assert_eq!(
            agent(mode, &edit("src/../../outside/x.py"), &project),
            ClaurstPermissionDecision::Deny(None)
        );
        assert_eq!(
            agent(
                mode,
                &request("Edit", PermissionCategory::Edit, json!({})),
                &project
            ),
            ClaurstPermissionDecision::Deny(None)
        );
    }
}

#[test]
fn out_of_workspace_edits_follow_the_mode_matrix_when_unsandboxed() {
    let (root, project) = workspace();
    let outside = root.path().join("outside/secret.py");
    let outside = edit(outside.to_str().unwrap());
    let unsandboxed = |mode| policy(mode).for_conversation(gent_types::Containment::Unsandboxed);
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &unsandboxed(PermissionMode::AskEveryTime),
            &outside,
            &project
        ),
        ASK
    );
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &unsandboxed(PermissionMode::AutoAcceptEdits),
            &outside,
            &project
        ),
        ClaurstPermissionDecision::Deny(None)
    );
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &unsandboxed(PermissionMode::Autonomous),
            &outside,
            &project
        ),
        ClaurstPermissionDecision::Deny(None)
    );
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &unsandboxed(PermissionMode::Bypass),
            &outside,
            &project
        ),
        ALLOW
    );
}

#[test]
fn commands_follow_the_shared_sandbox_rule_and_ask_without_an_os_sandbox() {
    let (_root, project) = workspace();
    for mode in [
        PermissionMode::AskEveryTime,
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        for command in ["python3 -m src.cli add apples 3", "rm -rf build"] {
            assert_eq!(
                agent(mode, &bash(command), &project),
                ASK,
                "{mode:?} {command}"
            );
        }
    }
    let mut granted = policy(PermissionMode::AskEveryTime);
    granted.allowed_categories = vec![PermissionCategory::Command];
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &granted,
            &bash("cargo test"),
            &project
        ),
        ALLOW
    );
    granted.mode = PermissionMode::Autonomous;
    assert_eq!(
        decide(
            AgentChatMode::Agent,
            &granted,
            &bash("cargo test"),
            &project
        ),
        ASK
    );
}

#[test]
fn network_and_other_tools_require_a_supported_explicit_grant() {
    let (_root, project) = workspace();
    let fetch = request(
        "WebFetch",
        PermissionCategory::Network,
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(agent(PermissionMode::Autonomous, &fetch, &project), ASK);
    assert_eq!(agent(PermissionMode::Bypass, &fetch, &project), ASK);
    let mut granted = policy(PermissionMode::AskEveryTime);
    granted.allowed_categories = vec![PermissionCategory::Network];
    assert_eq!(
        decide(AgentChatMode::Agent, &granted, &fetch, &project),
        ALLOW
    );
}

#[test]
fn ask_every_time_frees_reads_and_network_but_still_asks_for_edits_and_commands() {
    let (_root, project) = workspace();
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &read("src/app.py"), &project),
        ALLOW
    );
    let fetch = request(
        "WebFetch",
        PermissionCategory::Network,
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(agent(PermissionMode::AskEveryTime, &fetch, &project), ALLOW);
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &edit("src/app.py"), &project),
        ASK
    );
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &bash("ls"), &project),
        ASK
    );
}

#[test]
fn auto_accept_edits_frees_network_but_still_asks_for_commands() {
    let (_root, project) = workspace();
    let fetch = request(
        "WebFetch",
        PermissionCategory::Network,
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(
        agent(PermissionMode::AutoAcceptEdits, &fetch, &project),
        ALLOW
    );
    assert_eq!(
        agent(PermissionMode::AutoAcceptEdits, &bash("ls"), &project),
        ASK
    );
}

#[test]
fn persisted_broad_modes_cannot_bypass_missing_containment_with_exact_grants() {
    let (_root, project) = workspace();
    for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
        let mut granted = policy(mode);
        granted.allowed_tools = vec!["Read".into(), "Edit".into(), "WebFetch".into()];
        granted.allowed_categories = vec![
            PermissionCategory::Read,
            PermissionCategory::Edit,
            PermissionCategory::Network,
        ];
        for request in [
            read("src/app.py"),
            edit("src/app.py"),
            request("WebFetch", PermissionCategory::Network, json!({})),
        ] {
            assert_eq!(
                decide(AgentChatMode::Agent, &granted, &request, &project),
                ASK
            );
        }
    }
}
