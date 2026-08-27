use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, AutomationAction,
    AutomationDefinition, AutomationId, AutomationNotifications, AutomationTrigger, ContextPolicy,
    ConversationListItem, ConversationRunStatus, ConversationStatus, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use std::collections::BTreeMap;

use super::{ConversationView, UiCommand, UiEffect, UiRequest, UiRequestResult, UiState};
use crate::terminal::ConversationMetadata;

fn item(id: &str) -> ConversationListItem {
    ConversationListItem {
        conversation_id: id.into(),
        run_count: 1,
    }
}

fn automation(id: &str, enabled: bool) -> AutomationDefinition {
    AutomationDefinition {
        automation_id: AutomationId(id.into()),
        workspace_id: "workspace".into(),
        name: "Review workspace".into(),
        working_directory: "/workspace".into(),
        enabled,
        action: AutomationAction::Prompt {
            prompt: "Review the workspace".into(),
        },
        trigger: AutomationTrigger::Manual,
        condition: None,
        selection: AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen3-1-7b-q4-k-m".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
        chain_target: None,
        notifications: AutomationNotifications::default(),
        created_at: 0,
        updated_at: 0,
        last_run: None,
    }
}

#[test]
fn selection_is_clamped_and_empty_state_is_safe() {
    let mut state = UiState::new(vec![item("one"), item("two")]);
    assert_eq!(state.apply(UiCommand::SelectPrevious), UiEffect::Continue);
    assert_eq!(state.selected().unwrap().conversation_id, "one");
    assert_eq!(
        state.apply(UiCommand::SelectNext),
        UiEffect::Refresh("two".into())
    );
    assert_eq!(state.apply(UiCommand::SelectNext), UiEffect::Continue);
    assert_eq!(state.selected().unwrap().conversation_id, "two");
    let mut empty = UiState::new(Vec::new());
    assert!(empty.selected().is_none());
    assert_eq!(empty.apply(UiCommand::SelectNext), UiEffect::Continue);
}

#[test]
fn accepted_prompt_keeps_refreshing_before_a_provider_turn_starts() {
    let mut state = UiState::new(vec![item("one")]);
    state.apply_request(UiRequestResult {
        conversation: item("one"),
        parent_run_id: Some("run".into()),
        notice: "Gent is preparing the selected provider…".into(),
        permission_mode: None,
        session: None,
        awaiting_turn: Some(true),
    });
    assert!(state.awaiting_turn());
    state.apply_request(UiRequestResult {
        conversation: item("one"),
        parent_run_id: None,
        notice: "Canceled current work.".into(),
        permission_mode: None,
        session: None,
        awaiting_turn: Some(false),
    });
    assert!(!state.awaiting_turn());
}

#[test]
fn search_filters_titles_and_navigation_stays_inside_visible_results() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            title: Some("Release notes".into()),
            ..ConversationMetadata::default()
        },
    );
    metadata.insert(
        "two".into(),
        ConversationMetadata {
            recap: Some("Review the release candidate".into()),
            ..ConversationMetadata::default()
        },
    );
    metadata.insert(
        "three".into(),
        ConversationMetadata {
            title: Some("Research notes".into()),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one"), item("two"), item("three")])
        .with_chat_input(true)
        .with_metadata(metadata);
    for character in "/search release".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Refresh("one".into())
    );
    assert_eq!(state.visible_conversation_indices(), [0, 1]);
    assert_eq!(
        state.selected().map(|item| item.conversation_id.as_str()),
        Some("one")
    );
    assert_eq!(
        state.apply(UiCommand::SelectNext),
        UiEffect::Refresh("two".into())
    );
    assert_eq!(
        state.selected().map(|item| item.conversation_id.as_str()),
        Some("two")
    );
}

#[test]
fn session_open_uses_its_most_recent_conversation() {
    let session = gent_types::AgentChatSession {
        session_id: gent_types::AgentChatSessionId("session".into()),
        workspace_id: "workspace".into(),
        name: "Release work".into(),
        conversation_ids: vec!["one".into(), "two".into()],
        created_at: 0,
        updated_at: 1,
    };
    let mut state = UiState::new(vec![item("one"), item("two")]).with_sessions(vec![session]);
    state.apply(UiCommand::FocusSessions);
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Refresh("two".into())
    );
    assert_eq!(
        state.selected().map(|item| item.conversation_id.as_str()),
        Some("two")
    );
}

#[test]
fn creating_while_a_session_is_focused_preserves_that_session_binding() {
    let session = gent_types::AgentChatSession {
        session_id: gent_types::AgentChatSessionId("session".into()),
        workspace_id: "workspace".into(),
        name: "Release work".into(),
        conversation_ids: vec!["one".into()],
        created_at: 0,
        updated_at: 1,
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_sessions(vec![session]);
    state.apply(UiCommand::FocusSessions);
    assert!(matches!(
        state.apply(UiCommand::CreateConversation),
        UiEffect::Request(UiRequest::Create {
            session_id: Some(session_id),
            ..
        }) if session_id.0 == "session"
    ));
}

#[test]
fn selection_drops_status_that_belongs_to_the_previous_conversation() {
    let mut state =
        UiState::new(vec![item("one"), item("two")]).with_status(Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: Vec::new(),
        }));
    assert!(state.selected_status().is_some());
    state.apply(UiCommand::SelectNext);
    assert!(state.selected_status().is_none());
}

#[test]
fn quit_is_the_only_terminal_action() {
    let mut state = UiState::new(vec![item("one")]);
    assert_eq!(state.apply(UiCommand::SelectNext), UiEffect::Continue);
    assert_eq!(state.apply(UiCommand::Quit), UiEffect::Quit);
}

#[test]
fn enabled_input_emits_a_typed_request_without_doing_io() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply(UiCommand::Insert('h'));
    state.apply(UiCommand::Insert('i'));
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "hi".into(),
            attachments: Vec::new(),
        })
    );
    assert_eq!(state.input(), "hi");
}

#[test]
fn login_uses_the_selected_public_provider_without_leaving_the_terminal_state() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.set_selection(AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    });
    for character in "/login".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Login(AgentChatProvider::Codex)
    );
    assert!(state.input().is_empty());
}

#[test]
fn local_gent_login_is_a_clear_noop() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/login".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Gent uses the selected local model; no account login is needed.")
    );
}

#[test]
fn multiline_input_is_preserved_until_plain_enter_sends() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply(UiCommand::Insert('a'));
    state.apply(UiCommand::InsertNewline);
    state.apply(UiCommand::Insert('b'));
    assert_eq!(state.input(), "a\nb");
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send { text, .. }) if text == "a\nb"
    ));
}

#[test]
fn automation_picker_runs_the_selected_enabled_automation() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            automation_count: 1,
            automation_names: vec!["Review workspace".into()],
            automations: vec![automation("automation-review", true)],
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    for character in "/automation".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::RunAutomation {
            automation_id,
            conversation_id,
        }) if automation_id == "automation-review" && conversation_id == "one"
    ));
}

#[test]
fn pasted_escaped_file_path_attaches_and_is_sent_with_the_prompt() {
    let directory = tempfile::Builder::new()
        .prefix("gent attachment ")
        .tempdir()
        .unwrap();
    let path = directory.path().join("notes.txt");
    std::fs::write(&path, "attached").unwrap();
    let escaped = path.to_string_lossy().replace(' ', "\\ ");
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    assert_eq!(state.apply(UiCommand::Paste(escaped)), UiEffect::Continue);
    assert_eq!(state.attachment_count(), 1);
    for character in "read this".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "read this".into(),
            attachments: vec![path],
        })
    );
}

#[test]
fn pasted_file_url_attaches_the_local_file() {
    let directory = tempfile::Builder::new()
        .prefix("gent attachment url ")
        .tempdir()
        .unwrap();
    let path = directory.path().join("notes with spaces.txt");
    std::fs::write(&path, "attached").unwrap();
    let url = format!("file://{}", path.to_string_lossy().replace(' ', "%20"));
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    assert_eq!(state.apply(UiCommand::Paste(url)), UiEffect::Continue);
    assert_eq!(state.attachment_count(), 1);
    assert_eq!(
        state.notice(),
        Some("Attached notes with spaces.txt. Enter sends it with the prompt.")
    );
}

#[test]
fn slash_goal_is_bound_to_the_selected_conversation_and_exact_current_run() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "run-one".into(),
            parent_run_id: None,
            provider: "codex".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(status));
    for character in "/goal finish switching safely".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Goal {
            conversation_id: "one".into(),
            run_id: "run-one".into(),
            summary: "finish switching safely".into(),
        })
    );
}

#[test]
fn slash_git_reads_the_selected_workspace_projection() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            workspace_path: Some("/workspace".into()),
            git_branch: Some("feature/chat".into()),
            changed_file_count: Some(3),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    for character in "/git".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Git · feature/chat · 3 changed · /workspace")
    );
}

#[test]
fn slash_goal_never_guesses_a_run_binding() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/goal keep going".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Run status is unavailable; refusing to guess a /goal binding.")
    );
    assert_eq!(state.input(), "/goal keep going");
}

#[test]
fn empty_slash_goal_is_not_sent_as_a_provider_prompt() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/goal ".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("`/goal` requires a concise summary; no provider work was started")
    );
}

#[test]
fn slash_resume_reuses_the_selected_conversation_and_requires_no_provider_protocol() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/resume continue after restart".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "continue after restart".into(),
            attachments: Vec::new(),
        })
    );
    assert!(state.input().is_empty());
}

#[test]
fn slash_resume_without_text_refreshes_the_selected_conversation() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/resume".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Refresh("one".into())
    );
}

#[test]
fn slash_resume_can_select_a_named_conversation_or_send_to_it() {
    let mut state = UiState::new(vec![item("one"), item("two")]).with_chat_input(true);
    for character in "/resume two".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Refresh("two".into())
    );
    assert_eq!(state.selected().unwrap().conversation_id, "two");
    for character in "/resume one continue here".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "continue here".into(),
            attachments: Vec::new(),
        })
    );
}

#[test]
fn slash_selection_commands_update_provider_neutral_controls_without_sending_prompts() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/provider claurst".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.selection().provider,
        gent_types::AgentChatProvider::Claurst
    );
    assert_eq!(
        state.selection().model,
        gent_protocol::DEFAULT_LOCAL_MODEL_ID
    );
    for character in "/model qwen3-8b-q4-k-m".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.selection().model,
        gent_protocol::DEFAULT_LOCAL_MODEL_ID
    );
    assert!(state.notice().unwrap().contains("not available"));
    assert!(state.input().is_empty());
    for character in "/plan".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(state.selection().mode, gent_types::AgentChatMode::Plan);
}

#[test]
fn slash_model_switch_creates_the_selected_child_run() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "parent".into(),
            parent_run_id: None,
            provider: "claurst".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_local_model_ids(vec![
            gent_protocol::DEFAULT_LOCAL_MODEL_ID.into(),
            "qwen3-8b-q4-k-m".into(),
        ])
        .with_status(Some(status));
    for character in "/model qwen3-8b-q4-k-m".chars() {
        state.apply(UiCommand::Insert(character));
    }
    let UiEffect::Request(UiRequest::Switch {
        parent_run_id,
        selection,
        context_policy,
        ..
    }) = state.apply(UiCommand::SubmitPrompt)
    else {
        panic!("slash model must create the selected child run");
    };
    assert_eq!(parent_run_id, "parent");
    assert_eq!(selection.model, "qwen3-8b-q4-k-m");
    assert_eq!(context_policy, ContextPolicy::Preserve);
}

#[test]
fn claurst_model_picker_uses_the_daemon_catalogue() {
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_local_model_ids(vec!["gent-small".into(), "gent-large".into()]);
    for character in "/provider claurst".chars() {
        state.apply(UiCommand::Insert(character));
    }
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleModel);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    assert_eq!(state.selection().model, "gent-large");
}

#[test]
fn permission_posture_is_a_workspace_setting_not_a_chat_mode() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            workspace_id: Some("workspace-one".into()),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    for character in "/permissions edits".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::SetPermissionMode {
            mode: gent_types::PermissionMode::AutoAcceptEdits,
            ..
        })
    ));
    assert_eq!(state.selection().mode, AgentChatMode::Agent);
}

#[test]
fn permission_picker_saves_a_workspace_posture_without_switching_chat_mode() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            workspace_id: Some("workspace-one".into()),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    state.apply(UiCommand::CyclePermission);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::SetPermissionMode {
            mode: gent_types::PermissionMode::AutoAcceptEdits,
            ..
        })
    ));
    assert_eq!(state.selection().mode, AgentChatMode::Agent);
}

#[test]
fn permission_picker_requires_an_explicit_enter_for_bypass() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            workspace_id: Some("workspace-one".into()),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    state.apply(UiCommand::CyclePermission);
    for _ in 0..4 {
        state.apply(UiCommand::SelectNext);
    }
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::SetPermissionMode {
            mode: gent_types::PermissionMode::Bypass,
            bypass_consent: true,
            ..
        })
    ));
}

#[test]
fn fork_uses_the_same_context_preserving_child_run_path_as_a_selection_switch() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.parent_run_id = Some("run-one".into());
    for character in "/fork".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Switch {
            context_policy: gent_types::ContextPolicy::Preserve,
            ..
        })
    ));
}

#[test]
fn model_picker_includes_the_native_claude_and_codex_catalogues() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleModel);
    for _ in 0..3 {
        state.apply(UiCommand::SelectNext);
    }
    state.apply(UiCommand::SubmitPrompt);
    assert_eq!(state.selection().model, "opus");
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleModel);
    for _ in 0..8 {
        state.apply(UiCommand::SelectNext);
    }
    state.apply(UiCommand::SubmitPrompt);
    assert_eq!(state.selection().model, "gpt-5.3-codex-spark");
}

#[test]
fn codex_effort_picker_includes_ultra() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleEffort);
    for _ in 0..4 {
        state.apply(UiCommand::SelectNext);
    }
    state.apply(UiCommand::SubmitPrompt);
    assert_eq!(state.selection().effort, AgentChatEffort::Ultra);
}

#[test]
fn slash_new_creates_a_conversation_with_the_current_selection() {
    let mut state = UiState::new(Vec::new()).with_chat_input(true);
    for character in "/new".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Create { .. })
    ));
}

#[test]
fn document_picker_uses_the_selected_conversation_workspace() {
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "one".into(),
        ConversationMetadata {
            workspace_id: Some("workspace-alpha".into()),
            ..ConversationMetadata::default()
        },
    );
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_metadata(metadata);
    for character in "/documents".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert!(matches!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::ListDocuments { workspace_id, .. } if workspace_id == "workspace-alpha"
    ));
}

#[test]
fn unknown_slash_commands_remain_provider_prompts() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/provider-specific value".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "/provider-specific value".into(),
            attachments: Vec::new(),
        })
    );
}

#[test]
fn selection_switch_is_parent_bound_and_carries_each_control() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "parent".into(),
            parent_run_id: None,
            provider: "codex".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(status));
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleModel);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleEffort);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleMode);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SubmitPrompt);
    state.apply(UiCommand::CycleContext);
    let UiEffect::Request(UiRequest::Switch {
        conversation_id,
        parent_run_id,
        selection,
        context_policy,
    }) = state.apply(UiCommand::SwitchSelection)
    else {
        panic!("expected parent-bound selection switch");
    };
    assert_eq!(conversation_id, "one");
    assert_eq!(parent_run_id, "parent");
    assert_eq!(selection.model, "gpt-5.6");
    assert_eq!(context_policy, ContextPolicy::Clear);
}

#[test]
fn picker_selection_applies_a_context_preserving_switch_immediately() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "parent".into(),
            parent_run_id: None,
            provider: "claurst".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(status));
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::SelectNext);
    let UiEffect::Request(UiRequest::Switch {
        conversation_id,
        parent_run_id,
        selection,
        context_policy,
    }) = state.apply(UiCommand::SubmitPrompt)
    else {
        panic!("picker confirmation must create the selected child run");
    };
    assert_eq!(conversation_id, "one");
    assert_eq!(parent_run_id, "parent");
    assert_eq!(selection.provider, AgentChatProvider::Claude);
    assert_eq!(context_policy, ContextPolicy::Preserve);
}

#[test]
fn selection_switch_refuses_to_guess_an_unknown_parent() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    assert_eq!(state.apply(UiCommand::SwitchSelection), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Run status is unavailable; refusing to guess a switch parent.")
    );
}

#[test]
fn selection_switch_refuses_an_ambiguous_status_hierarchy() {
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: vec![
                ConversationRunStatus {
                    run_id: "first".into(),
                    parent_run_id: None,
                    provider: "claude".into(),
                    active_turn_id: None,
                    live_status: None,
                },
                ConversationRunStatus {
                    run_id: "second".into(),
                    parent_run_id: Some("first".into()),
                    provider: "codex".into(),
                    active_turn_id: None,
                    live_status: None,
                },
            ],
        }));
    assert_eq!(state.apply(UiCommand::SwitchSelection), UiEffect::Continue);
    assert!(state.notice().unwrap().contains("refusing to guess"));
}

#[test]
fn selection_switch_uses_the_daemon_current_run_after_a_prior_switch() {
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_view(Some(
            ConversationView::new("one", None, None)
                .with_current_run_id(Some("codex-child".into())),
        ));
    let UiEffect::Request(UiRequest::Switch { parent_run_id, .. }) =
        state.apply(UiCommand::SwitchSelection)
    else {
        panic!("the daemon current run must make a subsequent switch unambiguous");
    };
    assert_eq!(parent_run_id, "codex-child");
}

#[test]
fn refreshed_view_replaces_the_index_run_count_with_the_authoritative_hierarchy() {
    let mut state = UiState::new(vec![item("one")]);
    state.apply_view(ConversationView::new(
        "one",
        Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: vec![
                ConversationRunStatus {
                    run_id: "root".into(),
                    parent_run_id: None,
                    provider: "claude".into(),
                    active_turn_id: None,
                    live_status: None,
                },
                ConversationRunStatus {
                    run_id: "child".into(),
                    parent_run_id: Some("root".into()),
                    provider: "codex".into(),
                    active_turn_id: None,
                    live_status: None,
                },
            ],
        }),
        None,
    ));
    assert_eq!(state.selected().unwrap().run_count, 2);
}

#[test]
fn settled_response_clears_the_live_thinking_notice() {
    let mut state = UiState::new(vec![item("one")]);
    state.set_notice("Gent is thinking…".into());
    state.apply_view(ConversationView::new(
        "one",
        None,
        Some(NormalizedTranscriptPage {
            conversation_id: "one".into(),
            events: vec![NormalizedTranscriptEvent {
                cursor: 1,
                event_id: "event".into(),
                turn_id: "turn".into(),
                run_id: "run".into(),
                kind: NormalizedTranscriptKind::AssistantMessage,
                text: "Done".into(),
                is_partial: false,
            }],
            next_after_cursor: None,
        }),
    ));
    assert_eq!(state.notice(), Some("Ready for your next message."));
}

#[test]
fn loading_a_conversation_restores_its_durable_selection_before_switching() {
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::High,
        mode: AgentChatMode::Agent,
    };
    let view = ConversationView::new(
        "one",
        Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: vec![ConversationRunStatus {
                run_id: "run-one".into(),
                parent_run_id: None,
                provider: "codex".into(),
                active_turn_id: None,
                live_status: None,
            }],
        }),
        None,
    )
    .with_current_run_id(Some("run-one".into()))
    .with_selection(Some(selection.clone()));
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply_view(view);
    assert_eq!(state.selection(), &selection);
    assert_eq!(state.parent_run_id.as_deref(), Some("run-one"));
}

#[test]
fn refreshed_view_replaces_a_previous_selection_with_the_durable_switched_run() {
    let original = AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    };
    let switched = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::High,
        mode: AgentChatMode::Plan,
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_view(Some(
            ConversationView::new("one", None, None).with_selection(Some(original)),
        ));
    state.apply_view(
        ConversationView::new("one", None, None)
            .with_current_run_id(Some("codex-child".into()))
            .with_selection(Some(switched.clone())),
    );
    assert_eq!(state.selection(), &switched);
    assert_eq!(state.parent_run_id.as_deref(), Some("codex-child"));
}

#[test]
fn initial_conversation_view_restores_its_durable_selection() {
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "hermes-3".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Plan,
    };
    let mut state = UiState::new(vec![item("one")]);
    state = state.with_view(Some(
        ConversationView::new("one", None, None).with_selection(Some(selection.clone())),
    ));
    assert_eq!(state.selection(), &selection);
}

#[test]
fn page_navigation_moves_through_transcript_history_without_changing_conversation() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    assert_eq!(state.apply(UiCommand::ScrollOlder), UiEffect::Continue);
    assert_eq!(state.scroll_offset(), 8);
    assert_eq!(state.apply(UiCommand::ScrollNewer), UiEffect::Continue);
    assert_eq!(state.scroll_offset(), 0);
    assert_eq!(state.selected().unwrap().conversation_id, "one");
}
