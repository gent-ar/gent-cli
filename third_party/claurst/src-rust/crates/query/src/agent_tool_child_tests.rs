use std::sync::{Arc, Mutex};

use super::run_child;
use crate::{ChildEventSink, QueryEvent, QueryOutcome};

fn recording_sink() -> (ChildEventSink, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let sink: ChildEventSink = Arc::new(move |parent, event| {
        let record = record.clone();
        Box::pin(async move {
            tokio::task::yield_now().await;
            let line = match event {
                QueryEvent::ToolStart { tool_id, .. } => format!("{parent} start {tool_id}"),
                QueryEvent::ToolEnd { tool_id, .. } => format!("{parent} end {tool_id}"),
                _ => format!("{parent} other"),
            };
            record.lock().unwrap().push(line);
        })
    });
    (sink, seen)
}

#[tokio::test]
async fn a_childs_tool_start_and_end_reach_its_parent_before_the_child_returns() {
    let (sink, seen) = recording_sink();

    let outcome = run_child(
        Some(sink),
        Some("toolu_agent".into()),
        |events| async move {
            let events = events.expect("a child under a parent tool relays its events");
            assert_eq!(
                claurst_tools::PARENT_TOOL_CALL
                    .try_with(Clone::clone)
                    .ok()
                    .as_deref(),
                Some("toolu_agent")
            );
            events
                .send(QueryEvent::ToolStart {
                    tool_name: "Write".into(),
                    tool_id: "toolu_write".into(),
                    input_json: r#"{"file_path":"child.txt","content":"hi"}"#.into(),
                })
                .unwrap();
            events
                .send(QueryEvent::ToolEnd {
                    tool_name: "Write".into(),
                    tool_id: "toolu_write".into(),
                    result: "written".into(),
                    is_error: false,
                })
                .unwrap();
            QueryOutcome::Cancelled
        },
    )
    .await;

    assert!(matches!(outcome, QueryOutcome::Cancelled));
    assert_eq!(
        *seen.lock().unwrap(),
        [
            "toolu_agent start toolu_write",
            "toolu_agent end toolu_write"
        ]
    );
}
