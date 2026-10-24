use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
};

use super::{ClaudeTurnOptions, ClaudeTurnOptionsError};

#[test]
fn workspace_permissions_map_to_claude_permission_arguments() {
    for (permission, permission_mode) in [
        (PermissionMode::AskEveryTime, "manual"),
        (PermissionMode::AutoAcceptEdits, "acceptEdits"),
        (PermissionMode::Autonomous, "auto"),
        (PermissionMode::Bypass, "bypassPermissions"),
    ] {
        let options = ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Agent),
            permission,
        )
        .unwrap();
        let mut arguments = Vec::new();
        options.append_arguments(&mut arguments);
        assert_eq!(
            arguments,
            [
                "--model",
                "claude-sonnet",
                "--effort",
                "high",
                "--permission-prompt-tool",
                "stdio",
                "--permission-mode",
                permission_mode
            ]
        );
    }
}

#[test]
fn every_selected_effort_reaches_claude() {
    for (effort, expected) in [
        (AgentChatEffort::Low, "low"),
        (AgentChatEffort::Medium, "medium"),
        (AgentChatEffort::High, "high"),
        (AgentChatEffort::XHigh, "xhigh"),
        (AgentChatEffort::Max, "max"),
        (AgentChatEffort::Ultra, "max"),
    ] {
        let mut selection = selection(AgentChatMode::Agent);
        selection.effort = effort;
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection(&selection)
            .unwrap()
            .append_arguments(&mut arguments);
        assert!(
            arguments
                .windows(2)
                .any(|entry| entry[0] == "--effort" && entry[1] == expected)
        );
    }
}

#[test]
fn plan_mode_uses_claude_native_plan_permission_mode_for_every_posture() {
    for permission in [PermissionMode::AutoAcceptEdits, PermissionMode::Bypass] {
        let mut arguments = Vec::new();
        ClaudeTurnOptions::from_selection_with_permissions(
            &selection(AgentChatMode::Plan),
            permission,
        )
        .unwrap()
        .append_arguments(&mut arguments);
        assert!(
            arguments
                .windows(2)
                .any(|entry| entry[0] == "--permission-mode" && entry[1] == "plan")
        );
        assert!(!arguments.contains(&"--append-system-prompt".to_string()));
    }
}

#[test]
fn non_claude_and_malformed_selections_are_rejected() {
    let mut other = selection(AgentChatMode::Ask);
    other.provider = AgentChatProvider::Codex;
    assert_eq!(
        ClaudeTurnOptions::from_selection_with_permissions(&other, PermissionMode::AskEveryTime),
        Err(ClaudeTurnOptionsError::UnsupportedSelection)
    );
    let mut malformed = selection(AgentChatMode::Ask);
    malformed.model = "\0".into();
    assert_eq!(
        ClaudeTurnOptions::from_selection_with_permissions(
            &malformed,
            PermissionMode::AskEveryTime
        ),
        Err(ClaudeTurnOptionsError::InvalidModel)
    );
}

#[test]
fn appended_conversation_config_composes_with_the_mode_instruction() {
    let mut arguments = Vec::new();
    ClaudeTurnOptions::from_selection_with_permissions(
        &selection(AgentChatMode::Ask),
        PermissionMode::AskEveryTime,
    )
    .unwrap()
    .with_conversation_config(Some("Prefer terse replies.".into()), true, None, Vec::new())
    .append_arguments(&mut arguments);
    let append_index = arguments
        .iter()
        .position(|argument| argument == "--append-system-prompt")
        .unwrap();
    let appended = &arguments[append_index + 1];
    assert!(appended.contains("Answer and explain"));
    assert!(appended.contains("Prefer terse replies."));
    assert_eq!(
        arguments
            .iter()
            .filter(|argument| *argument == "--append-system-prompt")
            .count(),
        1
    );
    assert!(!arguments.contains(&"--system-prompt".to_string()));
}

#[test]
fn full_override_replaces_the_default_prompt_but_keeps_the_mode_instruction_appended() {
    let mut arguments = Vec::new();
    ClaudeTurnOptions::from_selection_with_permissions(
        &selection(AgentChatMode::Ask),
        PermissionMode::AskEveryTime,
    )
    .unwrap()
    .with_conversation_config(
        Some("You are a terse reviewer.".into()),
        false,
        None,
        Vec::new(),
    )
    .append_arguments(&mut arguments);
    assert!(arguments.windows(2).any(|entry| {
        entry[0] == "--append-system-prompt" && entry[1].contains("Answer and explain")
    }));
    assert!(
        arguments.windows(2).any(|entry| {
            entry[0] == "--system-prompt" && entry[1] == "You are a terse reviewer."
        })
    );
}

#[test]
fn max_turns_and_disallowed_tools_append_when_configured() {
    let mut arguments = Vec::new();
    ClaudeTurnOptions::from_selection_with_permissions(
        &selection(AgentChatMode::Agent),
        PermissionMode::AskEveryTime,
    )
    .unwrap()
    .with_conversation_config(
        None,
        false,
        Some(7),
        vec!["shell:rm".into(), "web:fetch".into()],
    )
    .append_arguments(&mut arguments);
    assert!(
        arguments
            .windows(2)
            .any(|entry| entry[0] == "--max-turns" && entry[1] == "7")
    );
    assert!(
        arguments
            .windows(2)
            .any(|entry| { entry[0] == "--disallowedTools" && entry[1] == "shell:rm,web:fetch" })
    );
}

#[test]
fn summary_turn_is_fresh_and_has_no_tools_or_persistence() {
    let mut arguments = Vec::new();
    ClaudeTurnOptions::summary("haiku")
        .unwrap()
        .append_arguments(&mut arguments);
    assert_eq!(
        arguments,
        [
            "--model",
            "haiku",
            "--safe-mode",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--max-turns",
            "1",
            "--no-session-persistence",
        ]
    );
}

fn selection(mode: AgentChatMode) -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "claude-sonnet".into(),
        effort: AgentChatEffort::High,
        mode,
    }
}

#[test]
fn thinking_setting_is_scoped_and_preserves_model_effort_and_permissions() {
    let base = ClaudeTurnOptions::from_selection(&selection(AgentChatMode::Agent)).unwrap();
    let mut ordinary = Vec::new();
    base.append_arguments(&mut ordinary);
    assert!(!ordinary.iter().any(|arg| arg == "--settings"));
    for enabled in [false, true] {
        let mut arguments = Vec::new();
        base.clone()
            .with_thinking_enabled(Some(enabled))
            .append_arguments(&mut arguments);
        assert_eq!(arguments[0], "--settings");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&arguments[1]).unwrap(),
            serde_json::json!({"alwaysThinkingEnabled": enabled})
        );
        assert_eq!(&arguments[2..], ordinary.as_slice());
    }
}
