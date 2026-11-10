use gent_types::{
    AgentChatEffort, AgentChatFailureCode, AgentChatMode, AgentChatProvider, AgentChatSelection,
    DurableTurnPhase, NormalizedTranscriptKind,
};
use ratatui::style::Color;

use super::{
    UiState,
    inline_items::scroll_items,
    inline_tests::{event, screen, state, status, timeline},
};

fn failed(
    provider: AgentChatProvider,
    code: Option<AgentChatFailureCode>,
    notice: &str,
) -> UiState {
    let mut notice = event(3, NormalizedTranscriptKind::Notice, notice);
    notice.failure_code = code;
    let events = vec![
        event(
            1,
            NormalizedTranscriptKind::UserMessage,
            "Fix the failing test.",
        ),
        event(
            2,
            NormalizedTranscriptKind::AssistantMessage,
            "Failed to authenticate: OAuth session expired and could not be refreshed",
        ),
        notice,
    ];
    let mut state = state(events, Vec::new(), status(false, false, true));
    let view = state
        .view
        .take()
        .unwrap()
        .with_timeline(Some(timeline(DurableTurnPhase::Failed)))
        .with_selection(Some(AgentChatSelection {
            provider,
            model: "fixture".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Agent,
        }));
    state.with_view(Some(view))
}

fn rendered(state: &UiState, width: u16, rows: u16) -> (Vec<(String, Option<Color>)>, String) {
    let lines = scroll_items(state, width)
        .into_iter()
        .flat_map(|item| item.lines)
        .map(|line| {
            let color = line.spans.last().and_then(|span| span.style.fg);
            (line.to_string(), color)
        })
        .collect();
    let (text, _, _) = screen(state, width, rows);
    (lines, text.lines().last().unwrap().to_owned())
}

#[test]
fn an_expired_sign_in_tells_the_user_how_to_sign_in_and_marks_the_footer() {
    for (provider, name) in [
        (AgentChatProvider::Claude, "Claude"),
        (AgentChatProvider::Codex, "Codex"),
    ] {
        let notice = format!("{name} sign-in expired. Sign in again to continue.");
        let state = failed(
            provider,
            Some(AgentChatFailureCode::ProviderAuthRequired),
            &notice,
        );
        for (width, rows) in [(80, 24), (120, 36)] {
            let (lines, footer) = rendered(&state, width, rows);
            let hint = format!("  Type /login {} to sign in again.", name.to_lowercase());
            assert!(
                lines.contains(&(format!("! {notice}"), Some(Color::Yellow))),
                "{lines:?}"
            );
            assert!(lines.contains(&(hint, Some(Color::Yellow))), "{lines:?}");
            assert!(
                lines
                    .iter()
                    .any(|(line, _)| line.contains("OAuth session expired")),
                "{lines:?}"
            );
            assert!(footer.contains("sign in needed"), "{footer}");
            assert!(!footer.contains("last turn failed"), "{footer}");
        }
    }
}

#[test]
fn an_ordinary_failure_keeps_the_failed_footer_without_a_login_hint() {
    let state = failed(
        AgentChatProvider::Claude,
        None,
        "Claude ended the turn with an error.",
    );
    for (width, rows) in [(80, 24), (120, 36)] {
        let (lines, footer) = rendered(&state, width, rows);
        assert!(
            lines.contains(&(
                "! Claude ended the turn with an error.".into(),
                Some(Color::Yellow)
            )),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|(line, _)| !line.contains("/login")),
            "{lines:?}"
        );
        assert!(footer.contains("last turn failed"), "{footer}");
        assert!(!footer.contains("sign in needed"), "{footer}");
    }
}
