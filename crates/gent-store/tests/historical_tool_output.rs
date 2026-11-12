use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, Ledger, NormalizedSessionBatchLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ConversationActivityFact, ConversationActivityScope,
    HostEpoch, NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle,
    NormalizedTranscriptAppend, NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
};

use gent_store::SqliteLedger;

#[path = "historical_tool_output/support.rs"]
mod support;
use support::{batch, prepared};

fn output(
    turn: &str,
    index: u64,
    tool: &str,
    text: &str,
    partial: bool,
    materialized: bool,
) -> NormalizedSessionBatch {
    let mut value = batch(turn.into());
    value.lifecycle_event_id = format!("output-{index}");
    value.lifecycle = NormalizedSessionLifecycle::Event {
        event: NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id: tool.into(),
            text: text.into(),
            is_partial: partial,
        },
    };
    let transcript = value.transcript.as_mut().unwrap();
    transcript.event_id = format!("tool-transcript-{index}");
    transcript.kind = NormalizedTranscriptKind::ToolActivity;
    transcript.text = text.into();
    transcript.is_partial = partial;
    value.activity_event_id = materialized.then(|| format!("tool-activity-{index}"));
    value.activity = materialized.then(|| ConversationActivityFact::ToolOutput {
        scope: ConversationActivityScope {
            conversation_id: "conversation-a".into(),
            run_id: "run-a".into(),
            turn_id: turn.into(),
            host_epoch: HostEpoch(1),
            cursor: 0,
        },
        tool_use_id: tool.into(),
        text: text.into(),
        is_partial: partial,
        source_lifecycle_event_id: None,
    });
    value
}

#[test]
fn historical_outputs_rehydrate_restore_tail_and_follow_without_mutating_any_fact() {
    let (ledger, turn) = prepared();
    let conversation = AgentChatConversationId("conversation-a".into());
    for (index, tool, text, partial, materialized) in [
        (1, "tool-1", "first", true, false),
        (2, "tool-1", "second", true, false),
        (3, "tool-1", "replacement", false, false),
        (4, "child-tool", "child result", false, false),
        (5, "new-tool", "new result", false, true),
    ] {
        ledger
            .append_normalized_session_batch(&output(
                &turn,
                index,
                tool,
                text,
                partial,
                materialized,
            ))
            .unwrap();
    }
    let before = ledger.read_event_page(0, 100).unwrap();
    let restored = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 64)
        .unwrap()
        .facts;
    assert_eq!(restored.len(), 5);
    assert!(
        restored
            .windows(2)
            .all(|facts| facts[0].scope().cursor < facts[1].scope().cursor)
    );
    assert!(
        matches!(&restored[0], ConversationActivityFact::ToolOutput { tool_use_id, text, is_partial: true, source_lifecycle_event_id: Some(source), .. }
        if tool_use_id == "tool-1" && text == "first" && source == "output-1")
    );
    assert!(
        matches!(&restored[2], ConversationActivityFact::ToolOutput { tool_use_id, text, is_partial: false, .. }
        if tool_use_id == "tool-1" && text == "replacement")
    );
    assert!(
        matches!(&restored[3], ConversationActivityFact::ToolOutput { tool_use_id, text, .. }
        if tool_use_id == "child-tool" && text == "child result")
    );
    assert!(matches!(
        &restored[4],
        ConversationActivityFact::ToolOutput {
            source_lifecycle_event_id: None,
            ..
        }
    ));
    let projection = ledger
        .agent_chat_projection_page(&conversation, 0, 100)
        .unwrap();
    let followed = projection
        .events
        .iter()
        .filter(|event| event.kind == "activity")
        .map(|event| {
            serde_json::from_value::<ConversationActivityFact>(event.payload["activity"].clone())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(followed, restored);
    let tail = ledger
        .agent_chat_projection_tail(&conversation, 100, 100)
        .unwrap();
    assert_eq!(
        tail.activity
            .iter()
            .map(|event| event.payload["activity"].clone())
            .collect::<Vec<_>>(),
        restored
            .iter()
            .map(|fact| serde_json::to_value(fact).unwrap())
            .collect::<Vec<_>>()
    );
    for event in projection
        .events
        .iter()
        .filter(|event| event.payload.get("derivedFromLifecycleEventId").is_some())
    {
        let source = event.payload["derivedFromLifecycleEventId"]
            .as_str()
            .unwrap();
        assert_eq!(event.source_event_id, format!("lifecycle:{source}"));
    }
    assert!(
        ledger
            .agent_chat_projection_page(&conversation, tail.cursor, 100)
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(ledger.read_event_page(0, 100).unwrap(), before);
}

#[test]
fn historical_activity_pagination_keeps_existing_cursor_identity_and_run_scope() {
    let (ledger, turn) = prepared();
    for index in 1..=4 {
        ledger
            .append_normalized_session_batch(&output(
                &turn,
                index,
                "same-tool",
                &format!("result-{index}"),
                index < 3,
                index == 4,
            ))
            .unwrap();
    }
    let mut cursor = 0;
    let mut facts = Vec::new();
    loop {
        let page = ledger
            .read_conversation_activity_page("conversation-a", "run-a", cursor, 1)
            .unwrap();
        assert!(page.facts.iter().all(|fact| fact.scope().cursor > cursor));
        facts.extend(page.facts);
        let Some(next) = page.next_after_cursor else {
            break;
        };
        assert!(next > cursor);
        cursor = next;
    }
    assert_eq!(facts.len(), 4);
    assert!(
        ledger
            .read_conversation_activity_page("conversation-other", "run-a", 0, 10)
            .unwrap()
            .facts
            .is_empty()
    );
    assert!(
        ledger
            .read_conversation_activity_page("conversation-a", "run-other", 0, 10)
            .unwrap()
            .facts
            .is_empty()
    );
    let conversation = AgentChatConversationId("conversation-a".into());
    let all = ledger
        .agent_chat_projection_page(&conversation, 0, 100)
        .unwrap();
    let mut cursor = 0;
    let mut paged = Vec::new();
    loop {
        let page = ledger
            .agent_chat_projection_page(&conversation, cursor, 1)
            .unwrap();
        assert!(page.events.iter().all(|event| event.cursor > cursor));
        paged.extend(page.events);
        let Some(next) = page.next_after_cursor else {
            break;
        };
        assert!(next > cursor);
        cursor = next;
    }
    assert_eq!(paged, all.events);
}

#[test]
fn historical_child_tool_output_keeps_its_existing_parent_attribution() {
    let (ledger, turn) = prepared();
    let mut start = batch(turn.clone());
    let tool = gent_types::ToolActivity {
        tool_use_id: "child-tool".into(),
        tool_name: "Read".into(),
        phase: gent_types::ToolPhase::Started,
        input: None,
        output_digest: None,
        parent_tool_use_id: Some("parent-tool".into()),
    };
    start.lifecycle = NormalizedSessionLifecycle::Signal {
        signal: gent_types::NormalizedLifecycleSignal::ToolActivity {
            activity: tool.clone(),
        },
    };
    start.transcript = None;
    start.activity = Some(ConversationActivityFact::ToolActivity {
        scope: ConversationActivityScope {
            conversation_id: "conversation-a".into(),
            run_id: "run-a".into(),
            turn_id: turn.clone(),
            host_epoch: HostEpoch(1),
            cursor: 0,
        },
        activity: tool,
    });
    ledger.append_normalized_session_batch(&start).unwrap();
    ledger
        .append_normalized_session_batch(&output(
            &turn,
            1,
            "child-tool",
            "child result",
            false,
            false,
        ))
        .unwrap();
    let restored = ledger
        .read_conversation_activity_page("conversation-a", "run-a", 0, 10)
        .unwrap();
    assert!(matches!(restored.facts.as_slice(), [
        ConversationActivityFact::ToolActivity { activity, .. },
        ConversationActivityFact::ToolOutput { tool_use_id, text, .. }
    ] if activity.tool_use_id == *tool_use_id && activity.parent_tool_use_id.as_deref() == Some("parent-tool") && text == "child result"));
}

#[test]
fn activity_pages_stay_scoped_and_ordered_across_interleaved_runs_and_conversations() {
    let (ledger, turn_a) = prepared();
    let turn_b = support::prepare_conversation(&ledger, "b");
    for index in 1..=6 {
        let (name, turn) = if index % 2 == 0 {
            ("b", &turn_b)
        } else {
            ("a", &turn_a)
        };
        let mut value = output(
            turn,
            index,
            "tool",
            &format!("{name}-{index}"),
            false,
            index == 5,
        );
        value.conversation_id = format!("conversation-{name}");
        value.run_id = format!("run-{name}");
        value.transcript.as_mut().unwrap().run_id = format!("run-{name}");
        if let Some(ConversationActivityFact::ToolOutput { scope, .. }) = value.activity.as_mut() {
            scope.conversation_id = format!("conversation-{name}");
            scope.run_id = format!("run-{name}");
        }
        ledger.append_normalized_session_batch(&value).unwrap();
    }
    for (name, texts) in [("a", ["a-1", "a-3", "a-5"]), ("b", ["b-2", "b-4", "b-6"])] {
        let conversation = format!("conversation-{name}");
        let run = format!("run-{name}");
        let all = ledger
            .read_conversation_activity_page(&conversation, &run, 0, 64)
            .unwrap()
            .facts;
        assert_eq!(
            all.iter()
                .map(|fact| match fact {
                    ConversationActivityFact::ToolOutput { text, scope, .. } => {
                        assert_eq!(
                            (scope.conversation_id.as_str(), scope.run_id.as_str()),
                            (conversation.as_str(), run.as_str())
                        );
                        text.as_str()
                    }
                    other => panic!("unexpected {other:?}"),
                })
                .collect::<Vec<_>>(),
            texts
        );
        let mut cursor = 0;
        let mut paged = Vec::new();
        while let Some(next) = {
            let page = ledger
                .read_conversation_activity_page(&conversation, &run, cursor, 1)
                .unwrap();
            paged.extend(page.facts);
            page.next_after_cursor
        } {
            cursor = next;
        }
        assert_eq!(paged, all);
        let after_first = all[0].scope().cursor;
        assert_eq!(
            ledger
                .read_conversation_activity_page(&conversation, &run, after_first, 64)
                .unwrap()
                .facts,
            all[1..]
        );
    }
}
