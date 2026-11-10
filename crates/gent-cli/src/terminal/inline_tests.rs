use std::collections::BTreeMap;

use gent_types::{
    ConversationActivityFact, ConversationActivityScope, ConversationAttentionStatus,
    ConversationErrorStatus, ConversationListItem, ConversationLiveStatus,
    ConversationProcessingStatus, ConversationRunStatus, ConversationStatus, ConversationTimeline,
    ConversationTimelineRun, ConversationWorkStatus, DurableTurnPhase, HostEpoch,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage, RunLiveStatus,
    ToolActivity, ToolPhase, TurnRecord,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

use super::{
    ConversationMetadata, ConversationView, UiState,
    inline_items::{scroll_items, spaced},
    inline_view::compose,
};

pub(super) fn scope(cursor: u64) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "c1".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

pub(super) fn tool(
    id: &str,
    name: &str,
    phase: ToolPhase,
    input: serde_json::Value,
) -> ConversationActivityFact {
    ConversationActivityFact::ToolActivity {
        scope: scope(1),
        activity: ToolActivity {
            tool_use_id: id.into(),
            tool_name: name.into(),
            phase,
            input: Some(input),
            output_digest: None,
            parent_tool_use_id: None,
        },
    }
}

pub(super) fn output(id: &str, text: &str) -> ConversationActivityFact {
    ConversationActivityFact::ToolOutput {
        scope: scope(2),
        tool_use_id: id.into(),
        text: text.into(),
        is_partial: false,
        source_lifecycle_event_id: None,
    }
}

pub(super) fn event(
    cursor: u64,
    kind: NormalizedTranscriptKind,
    text: &str,
) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn".into(),
        run_id: "run".into(),
        kind,
        text: text.into(),
        is_partial: false,
        failure_code: None,
        origin: None,
        attachments: Vec::new(),
    }
}

pub(super) fn status(active: bool, attention: bool, error: bool) -> ConversationStatus {
    ConversationStatus {
        conversation_id: "c1".into(),
        runs: vec![ConversationRunStatus {
            run_id: "run".into(),
            parent_run_id: None,
            provider: "claude".into(),
            active_turn_id: active.then(|| "turn".into()),
            live_status: Some(RunLiveStatus {
                run_id: "run".into(),
                host_epoch: HostEpoch(1),
                status: ConversationLiveStatus {
                    cursor: 1,
                    processing: if active {
                        ConversationProcessingStatus::Processing
                    } else {
                        ConversationProcessingStatus::Idle
                    },
                    attention: if attention {
                        ConversationAttentionStatus::Required
                    } else {
                        ConversationAttentionStatus::Clear
                    },
                    error: if error {
                        ConversationErrorStatus::Error
                    } else {
                        ConversationErrorStatus::Clear
                    },
                    subagent_work: ConversationWorkStatus::None,
                    command_work: ConversationWorkStatus::None,
                },
            }),
        }],
    }
}

pub(super) fn timeline(phase: DurableTurnPhase) -> ConversationTimeline {
    ConversationTimeline {
        conversation_id: "c1".into(),
        runs: vec![ConversationTimelineRun {
            run_id: "run".into(),
            parent_run_id: None,
            provider: "claude".into(),
            turns: vec![TurnRecord {
                turn_id: "turn".into(),
                conversation_id: "c1".into(),
                run_id: "run".into(),
                sequence: 1,
                phase,
            }],
            checkpoints: Vec::new(),
        }],
        artifacts: Vec::new(),
    }
}

pub(super) fn state(
    events: Vec<NormalizedTranscriptEvent>,
    facts: Vec<ConversationActivityFact>,
    status: ConversationStatus,
) -> UiState {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "c1".to_owned(),
        ConversationMetadata {
            title: Some("Fix the inventory".into()),
            workspace_id: Some("workspace".into()),
            workspace_path: Some(r"X:\work\ws1".into()),
            ..ConversationMetadata::default()
        },
    );
    let view = ConversationView::new(
        "c1",
        Some(status),
        Some(NormalizedTranscriptPage {
            conversation_id: "c1".into(),
            events,
            next_after_cursor: None,
        }),
    )
    .with_activity(Some(facts))
    .with_metadata(metadata["c1"].clone());
    UiState::new(vec![ConversationListItem {
        conversation_id: "c1".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_metadata(metadata)
    .with_view(Some(view))
}

pub(super) fn worked_turn() -> UiState {
    let boilerplate = "The file X:\\work\\ws1\\inventory\\store.py has been updated successfully. (file state is current in your context — no need to Read it back)";
    let events = vec![
        event(
            1,
            NormalizedTranscriptKind::UserMessage,
            "Fix the failing test.",
        ),
        event(
            2,
            NormalizedTranscriptKind::ToolActivity,
            "1\tclass Store:\n2\t    pass\n",
        ),
        event(3, NormalizedTranscriptKind::ToolActivity, "Exit code 0"),
        event(4, NormalizedTranscriptKind::ToolActivity, boilerplate),
        event(
            5,
            NormalizedTranscriptKind::ToolActivity,
            "a.py:1:foo\nb.py:2:foo\nc.py:3:foo",
        ),
        event(
            6,
            NormalizedTranscriptKind::AssistantMessage,
            "Fixed it.\n\n\n- first point\n\n- second point\n\n",
        ),
    ];
    let facts = vec![
        tool(
            "r",
            "Read",
            ToolPhase::Completed,
            json!({"file_path": "inventory/store.py"}),
        ),
        output("r", "1\tclass Store:\n2\t    pass\n"),
        tool(
            "b",
            "Bash",
            ToolPhase::Completed,
            json!({"command": "echo hi > a.txt"}),
        ),
        output("b", "Exit code 0"),
        tool(
            "e",
            "Edit",
            ToolPhase::Completed,
            json!({"file_path": "inventory/store.py", "old_string": "def low(self):\n    return qty < limit", "new_string": "def low(self):\n    return qty <= limit"}),
        ),
        output("e", boilerplate),
        tool("g", "Grep", ToolPhase::Completed, json!({"pattern": "foo"})),
        output("g", "a.py:1:foo\nb.py:2:foo\nc.py:3:foo"),
    ];
    state(events, facts, status(false, false, false))
}

pub(super) fn scrollback(state: &UiState) -> Vec<String> {
    let items = scroll_items(state, 120);
    spaced(&items.iter().collect::<Vec<_>>(), None)
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

pub(super) fn screen(state: &UiState, width: u16, rows: u16) -> (String, u16, u16) {
    let items = scroll_items(state, 120);
    let pending = items.iter().filter(|item| !item.done).collect::<Vec<_>>();
    let composition = compose(state, &pending, None, None, width);
    let height = composition.height(width, rows);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| composition.draw(frame, state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, height, composition.chrome_rows())
}

pub(super) fn mid_turn() -> UiState {
    let events = vec![
        event(1, NormalizedTranscriptKind::UserMessage, "Run the tests."),
        NormalizedTranscriptEvent {
            is_partial: true,
            ..event(
                2,
                NormalizedTranscriptKind::AssistantMessage,
                "Running the suite now",
            )
        },
    ];
    let facts = vec![tool(
        "b",
        "Bash",
        ToolPhase::Started,
        json!({"command": "python -m unittest"}),
    )];
    state(events, facts, status(true, false, false))
}
