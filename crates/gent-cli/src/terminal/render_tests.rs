use gent_types::{NormalizedTranscriptEvent, NormalizedTranscriptKind, PermissionMode};
use ratatui::{Terminal, backend::TestBackend};
use std::collections::BTreeMap;

use super::{
    active_turn_label, operational_chips, render, render_text::conversation_title, transcript_lines,
};
use crate::terminal::{UiState, state::UiCommand};

#[test]
fn terminal_is_a_chat_surface_not_a_raw_identifier_browser() {
    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render(frame, &UiState::new(Vec::new()).with_chat_input(true)))
        .unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(output.contains("Message"));
    assert!(output.contains("Workspace"));
    assert!(output.contains("Activity"));
    assert!(output.contains("Automations"));
    assert!(output.contains("No messages yet"));
    assert!(output.contains("Ctrl+N new"));
    assert!(output.contains("context 0%"));
}

#[test]
fn header_never_renders_a_partial_operational_chip() {
    let chips = operational_chips(&UiState::new(Vec::new()), 12)
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>();
    assert!(chips.len() <= 10);
    assert_eq!(chips, "[ idle ] ");
}

#[test]
fn header_keeps_the_selected_plan_mode_visible() {
    let mut state = UiState::new(Vec::new()).with_chat_input(true);
    state.set_selection(gent_types::AgentChatSelection {
        provider: gent_types::AgentChatProvider::Claurst,
        model: gent_protocol::DEFAULT_LOCAL_MODEL_ID.into(),
        effort: gent_types::AgentChatEffort::Medium,
        mode: gent_types::AgentChatMode::Plan,
    });
    let chips = operational_chips(&state, 120)
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>();
    assert!(chips.contains("[ planning ]"));
}

#[test]
fn title_uses_the_first_user_prompt_without_an_identifier() {
    let event = event(
        NormalizedTranscriptKind::UserMessage,
        "please organize the release notes",
        false,
    );
    assert_eq!(conversation_title(&[event]), "organize the release notes");
}

#[test]
fn conversation_rail_uses_the_latest_durable_preview() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "conversation".into(),
        crate::terminal::ConversationMetadata {
            title: Some("Release work".into()),
            preview: Some("The latest assistant response is visible here.".into()),
            ..crate::terminal::ConversationMetadata::default()
        },
    );
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            render(
                frame,
                &UiState::new(vec![gent_types::ConversationListItem {
                    conversation_id: "conversation".into(),
                    run_count: 1,
                }])
                .with_chat_input(true)
                .with_metadata(metadata),
            )
        })
        .unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(output.contains("latest assistant response"));
}

#[test]
fn narrow_terminal_uses_compact_workspace_facts() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "conversation".into(),
        crate::terminal::ConversationMetadata {
            workspace_path: Some("/Users/example/Clouseau/gent-cli".into()),
            git_branch: Some("main".into()),
            changed_file_count: Some(12),
            mcp_server_count: 2,
            mcp_server_names: vec!["gent-automations".into(), "gent-forge".into()],
            ..crate::terminal::ConversationMetadata::default()
        },
    );
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            render(
                frame,
                &UiState::new(vec![gent_types::ConversationListItem {
                    conversation_id: "conversation".into(),
                    run_count: 1,
                }])
                .with_chat_input(true)
                .with_metadata(metadata),
            )
        })
        .unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(output.contains("MCP · 2"));
    assert!(!output.contains("gent-automations"));
    assert!(output.contains("Git · main · 12"));
}

#[test]
fn thinking_is_compact_and_assistant_content_stays_readable() {
    let lines = transcript_lines(
        &[
            event(
                NormalizedTranscriptKind::Thinking,
                "private reasoning",
                false,
            ),
            event(
                NormalizedTranscriptKind::AssistantMessage,
                "Visible answer",
                false,
            ),
        ],
        false,
    );
    let output = lines
        .into_iter()
        .flat_map(|line| line.spans)
        .map(|span| span.content.into_owned())
        .collect::<String>();
    assert!(output.contains("thinking"));
    assert!(!output.contains("private reasoning"));
    assert!(output.contains("Visible answer"));
}

#[test]
fn local_compaction_outcomes_render_as_notices_in_the_transcript() {
    let lines = transcript_lines(
        &[
            event(NormalizedTranscriptKind::UserMessage, "/compact", false),
            event(
                NormalizedTranscriptKind::Notice,
                gent_types::PROVIDER_CONTEXT_COMPACTED_NOTICE,
                false,
            ),
            event(
                NormalizedTranscriptKind::Notice,
                gent_types::CONTEXT_COMPACTION_FALLBACK_NOTICE,
                false,
            ),
        ],
        false,
    );
    let output = lines
        .into_iter()
        .flat_map(|line| line.spans)
        .map(|span| span.content.into_owned())
        .collect::<String>();
    assert!(output.contains("Notice"));
    assert!(output.contains("Context compacted; the conversation continues"));
    assert!(output.contains("Gent could not compact this conversation's context"));
}

#[test]
fn visible_thinking_uses_the_provider_emitted_text() {
    let lines = transcript_lines(
        &[event(
            NormalizedTranscriptKind::Thinking,
            "considered",
            false,
        )],
        true,
    );
    let output = lines
        .into_iter()
        .flat_map(|line| line.spans)
        .map(|span| span.content.into_owned())
        .collect::<String>();
    assert!(output.contains("considered"));
}

#[test]
fn composer_shows_the_selected_workspace_permission_posture() {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "conversation".into(),
        crate::terminal::ConversationMetadata {
            permission_mode: PermissionMode::AutoAcceptEdits,
            ..crate::terminal::ConversationMetadata::default()
        },
    );
    let backend = TestBackend::new(160, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            render(
                frame,
                &UiState::new(vec![gent_types::ConversationListItem {
                    conversation_id: "conversation".into(),
                    run_count: 1,
                }])
                .with_chat_input(true)
                .with_metadata(metadata),
            )
        })
        .unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(output.contains("Permissions auto edits"));
}

#[test]
fn model_picker_is_a_navigable_chat_panel() {
    let mut state = UiState::new(Vec::new())
        .with_chat_input(true)
        .with_model_catalog(Some(crate::terminal::selection::tests::catalog()));
    state.apply(UiCommand::CycleModel);
    let backend = TestBackend::new(110, 28);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, &state)).unwrap();
    let output = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(output.contains("Model · Enter apply · Esc cancel"));
    assert!(output.contains(gent_protocol::DEFAULT_LOCAL_MODEL_ID));
}

#[test]
fn active_turn_uses_the_authoritative_waiting_work_label() {
    let state = UiState::new(vec![gent_types::ConversationListItem {
        conversation_id: "conversation".into(),
        run_count: 1,
    }])
    .with_status(Some(gent_types::ConversationStatus {
        conversation_id: "conversation".into(),
        runs: vec![gent_types::ConversationRunStatus {
            run_id: "run".into(),
            parent_run_id: None,
            provider: "claurst".into(),
            active_turn_id: Some("turn".into()),
            live_status: Some(gent_types::RunLiveStatus {
                run_id: "run".into(),
                host_epoch: gent_types::HostEpoch(1),
                status: gent_types::ConversationLiveStatus {
                    cursor: 1,
                    processing: gent_types::ConversationProcessingStatus::Idle,
                    attention: gent_types::ConversationAttentionStatus::Clear,
                    error: gent_types::ConversationErrorStatus::Clear,
                    subagent_work: gent_types::ConversationWorkStatus::Waiting,
                    command_work: gent_types::ConversationWorkStatus::None,
                },
            }),
        }],
    }));
    assert_eq!(active_turn_label(&state), "Waiting for subagents…");
}

fn event(
    kind: NormalizedTranscriptKind,
    text: &str,
    is_partial: bool,
) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor: 1,
        event_id: "event".into(),
        turn_id: "turn".into(),
        run_id: "run".into(),
        kind,
        text: text.into(),
        is_partial,
        origin: None,
        attachments: Vec::new(),
    }
}

fn rendered(state: &UiState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 90)).unwrap();
    terminal.draw(|frame| render(frame, state)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

#[test]
fn typing_a_slash_renders_the_catalog_palette_help_and_typed_rejections() {
    let mut state = UiState::new(vec![gent_types::ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_command_catalog(Some(crate::terminal::commands::tests::catalog_for(Some(
        gent_types::AgentChatProvider::Claurst,
    ))));
    for character in "/re".chars() {
        state.apply(UiCommand::Insert(character));
    }
    let palette = rendered(&state);
    assert!(palette.contains("Commands"));
    assert!(palette.contains("/resume [conversation-id] — Open a conversation"));
    assert!(!palette.contains("/rename"));
    for character in "sume x".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert!(!rendered(&state).contains("Open a conversation"));
    state.replace_input("/nope".into());
    state.apply(UiCommand::SubmitPrompt);
    assert!(rendered(&state).contains("/nope is not a command in this conversation's catalog"));
    state.replace_input("/comp".into());
    assert!(rendered(&state).contains("/compact — Compact the conversation context"));
    state.replace_input(String::new());
    state.apply(UiCommand::ToggleHelp);
    let help = rendered(&state);
    assert!(help.contains("/fork — Fork this conversation into a new one"));
    assert!(!help.contains("/btw"));
}
