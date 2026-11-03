use super::{CodexTurnOptions, turn_parameters};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

#[test]
fn every_native_turn_explicitly_requests_provider_exposed_summary_auto() {
    for mode in [
        AgentChatMode::Ask,
        AgentChatMode::Plan,
        AgentChatMode::Agent,
    ] {
        let options = CodexTurnOptions::from_selection(&selection(mode), None).unwrap();
        assert_eq!(
            turn_parameters(&options, "thread", "prompt", &[])["summary"],
            "auto"
        );
    }
    let summary = CodexTurnOptions::summary("gpt-5.6-luna").unwrap();
    assert_eq!(
        turn_parameters(&summary, "thread", "summarize", &[])["summary"],
        "auto"
    );
}

#[test]
fn plan_mode_is_codex_native_collaboration_mode_without_a_prompt_prefix() {
    let options = CodexTurnOptions::from_selection(&selection(AgentChatMode::Plan), None)
        .unwrap()
        .with_conversation_config(Some("Prefer terse replies.".into()), true);
    let parameters = turn_parameters(&options, "thread", "do the thing", &[]);
    assert_eq!(
        parameters["collaborationMode"],
        serde_json::json!({"mode": "plan", "settings": {"model": "gpt-5.6", "reasoning_effort": "high", "developer_instructions": null}})
    );
    assert_eq!(
        parameters["input"][0]["text"],
        "Prefer terse replies.\n\nUser request:\ndo the thing"
    );
    let agent = CodexTurnOptions::from_selection(&selection(AgentChatMode::Agent), None).unwrap();
    assert!(
        turn_parameters(&agent, "thread", "do the thing", &[])
            .get("collaborationMode")
            .is_none()
    );
}

#[test]
fn a_non_append_system_prompt_is_ignored_at_the_wire_layer() {
    let options = CodexTurnOptions::from_selection(&selection(AgentChatMode::Agent), None)
        .unwrap()
        .with_conversation_config(Some("Replace everything.".into()), false);
    let parameters = turn_parameters(&options, "thread", "do the thing", &[]);
    assert_eq!(parameters["input"][0]["text"], "do the thing");
}

fn selection(mode: AgentChatMode) -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::High,
        mode,
    }
}

#[test]
fn summary_turn_never_requests_approval_or_network_access() {
    let options = CodexTurnOptions::summary("gpt-5.6-luna").unwrap();
    let parameters = turn_parameters(&options, "thread", "prompt", &[]);
    assert_eq!(parameters["approvalPolicy"], "never");
    assert_eq!(parameters["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(parameters["sandboxPolicy"]["networkAccess"], false);
    assert_eq!(parameters["effort"], "low");
}

#[test]
fn permission_posture_changes_approval_without_replacing_plan_intent() {
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Plan,
    };
    let options = CodexTurnOptions::from_selection_with_permissions(
        &selection,
        Some("/workspace"),
        PermissionMode::Bypass,
    )
    .unwrap();
    let parameters = turn_parameters(&options, "thread", "prompt", &[]);
    assert_eq!(parameters["approvalPolicy"], "never");
    assert_eq!(parameters["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(parameters["collaborationMode"]["mode"], "plan");
}

#[test]
fn codex_preserves_its_highest_supported_effort_level() {
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6-sol".into(),
        effort: AgentChatEffort::Ultra,
        mode: AgentChatMode::Agent,
    };
    let options = CodexTurnOptions::from_selection_with_permissions(
        &selection,
        Some("/workspace"),
        PermissionMode::AskEveryTime,
    )
    .unwrap();
    assert_eq!(
        turn_parameters(&options, "thread", "prompt", &[])["effort"],
        "ultra"
    );
}

#[test]
fn permission_matrix_preserves_interaction_mode_through_new_and_resumed_threads() {
    use super::super::{CodexAppServerSession, CodexSessionConfig, CodexSessionIngress};
    use serde_json::{Value, json};

    for mode in [
        AgentChatMode::Ask,
        AgentChatMode::Plan,
        AgentChatMode::Agent,
    ] {
        for (permission, approval) in [
            (PermissionMode::AskEveryTime, "untrusted"),
            (PermissionMode::AutoAcceptEdits, "untrusted"),
            (PermissionMode::Autonomous, "on-request"),
            (PermissionMode::Bypass, "never"),
        ] {
            for resume in [false, true] {
                let options = CodexTurnOptions::from_selection_with_permissions(
                    &selection(mode),
                    Some("/workspace"),
                    permission,
                )
                .unwrap();
                let (mut session, _) = CodexAppServerSession::start(CodexSessionConfig {
                    working_directory: Some("/workspace".into()),
                    resume_thread_id: resume.then(|| "thread".into()),
                    turn_options: options,
                    mcp_servers: None,
                })
                .unwrap();
                let CodexSessionIngress::Send(frames) =
                    session.receive(&json!({"id": 1, "result": {}})).unwrap()
                else {
                    panic!("expected thread request")
                };
                let thread: Value = serde_json::from_slice(&frames[1]).unwrap();
                assert_eq!(
                    thread["method"],
                    if resume {
                        "thread/resume"
                    } else {
                        "thread/start"
                    }
                );
                session
                    .receive(&json!({"id": 2, "result": {"thread": {"id": "thread"}}}))
                    .unwrap();
                let turn: Value =
                    serde_json::from_slice(&session.start_turn("prompt").unwrap()).unwrap();
                assert_eq!(turn["method"], "turn/start");
                let parameters = &turn["params"];
                assert_eq!(parameters["approvalPolicy"], approval);
                let expected = match (mode, permission) {
                    (AgentChatMode::Ask | AgentChatMode::Plan, _) => "readOnly",
                    (AgentChatMode::Agent, PermissionMode::Bypass) => "dangerFullAccess",
                    (AgentChatMode::Agent, _) => "workspaceWrite",
                };
                assert_eq!(
                    parameters["sandboxPolicy"]["type"], expected,
                    "{mode:?}/{permission:?}/resume={resume}"
                );
                if expected != "dangerFullAccess" {
                    assert_eq!(parameters["sandboxPolicy"]["networkAccess"], false);
                }
                if expected == "workspaceWrite" {
                    assert_eq!(
                        parameters["sandboxPolicy"]["writableRoots"],
                        json!(["/workspace"])
                    );
                }
                assert_eq!(
                    parameters.get("collaborationMode").is_some(),
                    mode == AgentChatMode::Plan
                );
            }
        }
    }
}
