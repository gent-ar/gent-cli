use gent_types::{DurableTurnPhase, HostEpoch, PermissionMode};
use serde_json::json;

use super::{
    UiState,
    inline_tests::{mid_turn, screen, state, status, timeline},
    state::{UiCommand, UiEffect, UiRequest},
};

#[test]
fn chrome_takes_at_most_five_rows_and_the_live_area_shows_the_running_tool() {
    for (width, rows) in [(80, 24), (120, 36)] {
        let state = mid_turn();
        let (text, height, chrome) = screen(&state, width, rows);
        assert!(chrome < 5, "chrome {chrome} at {width}x{rows}");
        assert!(
            text.contains("● Bash python -m unittest  running…"),
            "{text}"
        );
        assert!(text.contains("Running the suite now"), "{text}");
        assert!(text.contains("ctrl+c to interrupt"), "{text}");
        assert!(height <= rows);
        let last = text.lines().last().unwrap();
        assert!(last.contains("haiku") || last.contains("·"), "{last}");
    }
}

fn permission_state() -> UiState {
    let request = gent_types::PermissionDecisionRequest {
        binding: gent_types::PermissionDecisionBinding {
            decision_id: gent_types::AgentChatDecisionId("decision-1".into()),
            request_idempotency_key: "key".into(),
            conversation_id: gent_types::AgentChatConversationId("c1".into()),
            run_id: gent_types::AgentChatRunId("run".into()),
            turn_id: "turn".into(),
            policy_id: "policy".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: gent_types::PermissionRequestDigest("digest".into()),
        },
        request: gent_types::PermissionRequest::new(
            "Command".into(),
            gent_types::PermissionCategory::Command,
            Some(json!({"command": "cargo build -p gentd --release"})),
            None,
        ),
    };
    let mut state = mid_turn();
    let view = state
        .view
        .take()
        .unwrap()
        .with_pending_permission(Some(request));
    state.with_view(Some(view))
}

#[test]
fn permission_menu_shows_the_command_and_answers_with_a_digit() {
    let mut state = permission_state();
    let (text, _, _) = screen(&state, 80, 24);
    assert!(text.contains("Run this command?"), "{text}");
    assert!(text.contains("cargo build -p gentd --release"), "{text}");
    assert!(text.contains("1. Approve once"), "{text}");
    assert!(
        text.contains("2. Always allow this tool (Command)"),
        "{text}"
    );
    assert!(text.contains("4. Deny"), "{text}");
    assert!(!text.contains("Command · Command"), "{text}");
    let UiEffect::Request(UiRequest::Permission { response }) = state.apply(UiCommand::Insert('2'))
    else {
        panic!("digit 2 must answer the permission");
    };
    assert_eq!(
        response.response,
        gent_types::PermissionDecisionResponseKind::ApproveExactTool
    );
    let mut state = permission_state();
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    let UiEffect::Request(UiRequest::Permission { response }) =
        state.apply(UiCommand::SubmitPrompt)
    else {
        panic!("Enter must answer the highlighted choice");
    };
    assert_eq!(
        response.response,
        gent_types::PermissionDecisionResponseKind::Deny
    );
}

fn footer(state: &UiState) -> String {
    let (text, _, _) = screen(state, 120, 36);
    text.lines().last().unwrap().to_owned()
}

#[test]
fn footer_says_applies_next_turn_exactly_while_a_turn_is_active_or_waiting() {
    let idle = state(Vec::new(), Vec::new(), status(false, false, false));
    let active = state(Vec::new(), Vec::new(), status(true, false, false));
    let waiting = state(Vec::new(), Vec::new(), status(false, true, false));
    assert!(
        !footer(&idle).contains("applies next turn"),
        "{}",
        footer(&idle)
    );
    assert!(
        footer(&active).contains("applies next turn"),
        "{}",
        footer(&active)
    );
    assert!(
        footer(&waiting).contains("applies next turn"),
        "{}",
        footer(&waiting)
    );
}

#[test]
fn only_a_failed_latest_turn_shows_an_error() {
    let healthy = state(Vec::new(), Vec::new(), status(false, false, true));
    let healthy = {
        let mut value = healthy;
        let view = value
            .view
            .take()
            .unwrap()
            .with_timeline(Some(timeline(DurableTurnPhase::Completed)));
        value.with_view(Some(view))
    };
    let (text, _, _) = screen(&healthy, 120, 36);
    assert!(!text.to_lowercase().contains("error"), "{text}");
    assert!(!text.contains("failed"), "{text}");
    let mut failed = healthy;
    let view = failed
        .view
        .take()
        .unwrap()
        .with_timeline(Some(timeline(DurableTurnPhase::Failed)));
    let failed = failed.with_view(Some(view));
    assert!(
        footer(&failed).contains("last turn failed"),
        "{}",
        footer(&failed)
    );
}

#[test]
fn shift_tab_saves_the_next_mode_and_bypass_needs_confirmation() {
    let mut state = state(Vec::new(), Vec::new(), status(true, false, false));
    let UiEffect::Request(UiRequest::SetPermissionMode {
        mode,
        bypass_consent,
        ..
    }) = state.apply(UiCommand::CyclePermissionMode)
    else {
        panic!("shift+tab must save a mode even while a turn runs");
    };
    assert_eq!(mode, PermissionMode::AutoAcceptEdits);
    assert!(!bypass_consent);
    state.metadata.get_mut("c1").unwrap().permission_mode = PermissionMode::Autonomous;
    assert_eq!(
        state.apply(UiCommand::CyclePermissionMode),
        UiEffect::Continue
    );
    let (text, _, _) = screen(&state, 120, 36);
    assert!(text.contains("Enter confirms bypass"), "{text}");
    let UiEffect::Request(UiRequest::SetPermissionMode {
        mode,
        bypass_consent,
        ..
    }) = state.apply(UiCommand::SubmitPrompt)
    else {
        panic!("Enter must confirm bypass");
    };
    assert_eq!(mode, PermissionMode::Bypass);
    assert!(bypass_consent);
}

#[test]
fn ctrl_c_interrupts_a_turn_and_quits_only_when_pressed_twice_while_idle() {
    let mut idle = state(Vec::new(), Vec::new(), status(false, false, false));
    assert_eq!(idle.apply(UiCommand::Interrupt), UiEffect::Continue);
    assert_eq!(idle.apply(UiCommand::Interrupt), UiEffect::Quit);
    let mut active = state(Vec::new(), Vec::new(), status(true, false, false));
    assert!(matches!(
        active.apply(UiCommand::Interrupt),
        UiEffect::Request(UiRequest::Interrupt { .. })
    ));
}
