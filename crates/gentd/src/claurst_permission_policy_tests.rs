use std::path::{Path, PathBuf};

use gent_types::{
    AgentChatMode, PermissionCategory, PermissionDenialReason, PermissionMode, PermissionRequest,
    PolicyRecord, PolicyScope,
};
use serde_json::json;

use super::{ClaurstPermissionDecision, decide};

const ALLOW: ClaurstPermissionDecision = ClaurstPermissionDecision::Allow;
const ASK: ClaurstPermissionDecision = ClaurstPermissionDecision::Ask;
const READ_ONLY: ClaurstPermissionDecision =
    ClaurstPermissionDecision::Deny(Some(PermissionDenialReason::ReadOnlyMode));

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
        allowed_tools: vec![],
        allowed_categories: vec![],
    }
}

fn request(
    tool: &str,
    category: PermissionCategory,
    input: serde_json::Value,
) -> PermissionRequest {
    PermissionRequest::new(tool.into(), category, Some(input), None)
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
fn ask_and_plan_deny_changes_with_a_typed_reason_whatever_the_policy_grants() {
    let (_root, project) = workspace();
    let mut granted = policy(PermissionMode::Bypass);
    granted.allowed_categories = vec![PermissionCategory::Command, PermissionCategory::Edit];
    granted.allowed_tools = vec!["Bash".into(), "Edit".into()];
    for mode in [AgentChatMode::Ask, AgentChatMode::Plan] {
        assert_eq!(
            decide(mode, &granted, &edit("src/app.py"), &project),
            READ_ONLY
        );
        assert_eq!(decide(mode, &granted, &bash("ls"), &project), READ_ONLY);
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
}

#[test]
fn workspace_reads_never_ask_but_reads_outside_it_do_unless_bypassed() {
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
    assert_eq!(
        agent(PermissionMode::Bypass, &absolute_glob, &project),
        ALLOW
    );
}

#[test]
fn edits_are_unattended_only_inside_the_workspace_in_an_edit_accepting_mode() {
    let (root, project) = workspace();
    let inside = project.join("src/new.py");
    std::os::unix::fs::symlink(root.path().join("outside"), project.join("escape")).unwrap();
    assert_eq!(
        agent(PermissionMode::AskEveryTime, &edit("src/app.py"), &project),
        ASK
    );
    for mode in [
        PermissionMode::AutoAcceptEdits,
        PermissionMode::Autonomous,
        PermissionMode::Bypass,
    ] {
        assert_eq!(agent(mode, &edit("src/app.py"), &project), ALLOW);
        assert_eq!(
            agent(mode, &edit(inside.to_str().unwrap()), &project),
            ALLOW
        );
        assert_eq!(agent(mode, &edit("src/../../outside/x.py"), &project), ASK);
        assert_eq!(agent(mode, &edit("escape/x.py"), &project), ASK);
        assert_eq!(
            agent(
                mode,
                &request("Edit", PermissionCategory::Edit, json!({})),
                &project
            ),
            ASK
        );
    }
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
fn network_and_other_tools_are_unattended_only_when_bypassed_or_granted() {
    let (_root, project) = workspace();
    let fetch = request(
        "WebFetch",
        PermissionCategory::Network,
        json!({ "url": "https://example.com" }),
    );
    assert_eq!(agent(PermissionMode::Autonomous, &fetch, &project), ASK);
    assert_eq!(agent(PermissionMode::Bypass, &fetch, &project), ALLOW);
    let mut granted = policy(PermissionMode::AskEveryTime);
    granted.allowed_categories = vec![PermissionCategory::Network];
    assert_eq!(
        decide(AgentChatMode::Agent, &granted, &fetch, &project),
        ALLOW
    );
}
