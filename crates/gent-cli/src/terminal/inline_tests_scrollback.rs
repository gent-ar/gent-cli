use gent_types::{ConversationActivityFact, NormalizedTranscriptKind};

use super::{
    inline_items::{scroll_items, spaced},
    inline_tests::{event, mid_turn, scope, screen, scrollback, state, status, worked_turn},
};

#[test]
fn tool_rows_show_their_target_result_and_inline_diff() {
    let lines = scrollback(&worked_turn());
    let text = lines.join("\n");
    assert!(
        text.contains("● Read inventory/store.py  (2 lines)"),
        "{text}"
    );
    assert!(text.contains("● Bash echo hi > a.txt  ✓ exit 0"), "{text}");
    assert!(text.contains("● Edit inventory/store.py  +1 -1"), "{text}");
    assert!(text.contains("    -     return qty < limit"), "{text}");
    assert!(text.contains("    +     return qty <= limit"), "{text}");
    assert!(
        text.find("-     return qty < limit") < text.find("+     return qty <= limit"),
        "{text}"
    );
    assert!(text.contains("● Grep \"foo\"  3 matches"), "{text}");
    assert!(!text.contains("has been updated successfully"), "{text}");
    assert!(!text.contains("1\tclass Store"), "{text}");
}

#[test]
fn formatted_scrollback_never_has_consecutive_blank_lines() {
    let lines = scrollback(&worked_turn());
    assert!(lines.iter().any(|line| line.contains("second point")));
    for pair in lines.windows(2) {
        assert!(
            !(pair[0].is_empty() && pair[1].is_empty()),
            "{}",
            lines.join("\n")
        );
    }
}

#[test]
fn queued_prompts_show_their_text_in_the_live_area_not_the_scrollback() {
    let mut queued = event(
        3,
        NormalizedTranscriptKind::UserMessage,
        "also check the pipe path",
    );
    queued.event_id = "user:message-2".into();
    queued.turn_id = "turn-2".into();
    let mut state = mid_turn();
    let mut events = state.selected_transcript().to_vec();
    events.push(queued);
    let mut facts = state.selected_activity().to_vec();
    facts.push(ConversationActivityFact::PromptQueued {
        scope: scope(9),
        message_id: "message-2".into(),
    });
    state = self::state(events, facts, status(true, false, false));
    assert!(!scrollback(&state).join("\n").contains("pipe path"));
    let (text, _, _) = screen(&state, 120, 36);
    assert!(
        text.contains("◷ Queued  also check the pipe path"),
        "{text}"
    );
}

#[test]
fn a_prompt_released_after_a_turn_replays_right_before_its_own_reply() {
    let mut second = event(2, NormalizedTranscriptKind::UserMessage, "say banana");
    second.turn_id = "turn-2".into();
    let mut banana = event(4, NormalizedTranscriptKind::AssistantMessage, "banana");
    banana.turn_id = "turn-2".into();
    let state = state(
        vec![
            event(1, NormalizedTranscriptKind::UserMessage, "sleep first"),
            second,
            event(3, NormalizedTranscriptKind::AssistantMessage, "slept"),
            banana,
        ],
        Vec::new(),
        status(false, false, false),
    );
    let text = scrollback(&state).join("\n");
    let position = |needle: &str| text.find(needle).unwrap();
    assert!(position("sleep first") < position("slept"), "{text}");
    assert!(position("slept") < position("say banana"), "{text}");
    assert!(position("say banana") < position("● banana"), "{text}");
}

#[test]
fn rendered_paragraphs_are_separated_by_exactly_one_blank_row() {
    let text = "Refactoring complete:
- Created `rates.py`
- Added a price column

All 4 tests pass.";
    let state = state(
        vec![event(1, NormalizedTranscriptKind::AssistantMessage, text)],
        Vec::new(),
        status(false, false, false),
    );
    let items = scroll_items(&state, 120);
    let paragraph =
        ratatui::widgets::Paragraph::new(spaced(&items.iter().collect::<Vec<_>>(), None))
            .wrap(ratatui::widgets::Wrap { trim: false });
    let height = u16::try_from(paragraph.line_count(120)).unwrap();
    let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 120, height));
    ratatui::widgets::Widget::render(paragraph, buffer.area, &mut buffer);
    let rows = (0..height)
        .map(|y| {
            (0..120)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 5, "{rows:#?}");
    assert_eq!(rows[3], "", "{rows:#?}");
    assert_eq!(rows[4], "  All 4 tests pass.", "{rows:#?}");
}
