use super::tool_output_text;
use serde_json::json;

struct OutputStdio;

#[test]
fn provider_failure_preserves_its_message_without_raw_metadata() {
    let mut transport = super::super::ClaurstAcpTransport::new(OutputStdio);
    let update = json!({"update": {
        "sessionUpdate": "agent_error", "message": "Request failed",
        "metadata": {"credential": "not presentation content"}
    }});
    assert!(matches!(
        transport.session_update_fact(Some(&update)),
        Some(super::super::ClaurstAcpFact::Event(gent_types::NormalizedProviderEvent::ProviderFailure {
            message, ..
        })) if message == "Request failed"
    ));
}

impl super::super::ClaurstAcpStdio for OutputStdio {
    fn write_frame(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}

#[test]
fn running_tool_updates_retain_replaceable_result_content() {
    let mut transport = super::super::ClaurstAcpTransport::new(OutputStdio);
    let start = json!({"update": {
        "sessionUpdate": "tool_call", "toolCallId": "tool-1",
        "title": "Search", "status": "in_progress"
    }});
    assert!(transport.session_update_fact(Some(&start)).is_some());
    let update = json!({"update": {
        "sessionUpdate": "tool_call_update", "toolCallId": "tool-1",
        "content": [{"type": "content", "content": {"type": "text", "text": "one match"}}]
    }});
    assert!(matches!(
        transport.session_update_fact(Some(&update)),
        Some(super::super::ClaurstAcpFact::Event(gent_types::NormalizedProviderEvent::ToolOutputDelta {
            tool_use_id, text, is_partial: false,
        })) if tool_use_id == "tool-1" && text == "one match"
    ));
    assert!(matches!(
        transport.queued.pop_front(),
        Some(super::super::ClaurstAcpFact::Lifecycle(
            gent_types::NormalizedLifecycleSignal::ToolActivity {
                activity: gent_types::ToolActivity {
                    phase: gent_types::ToolPhase::Started,
                    ..
                },
            }
        ))
    ));
    let clear = json!({"update": {
        "sessionUpdate": "tool_call_update", "toolCallId": "tool-1", "content": []
    }});
    assert!(matches!(
        transport.session_update_fact(Some(&clear)),
        Some(super::super::ClaurstAcpFact::Event(gent_types::NormalizedProviderEvent::ToolOutputDelta {
            text, is_partial: false, ..
        })) if text.is_empty()
    ));
}

#[test]
fn structured_tool_results_remain_readable() {
    let output = json!({"matches": [{"path": "src/main.rs", "line": 12}], "extra": true});
    let text = tool_output_text(None, Some(&output));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        output
    );
    assert_eq!(
        tool_output_text(None, Some(&json!(["one", "two"]))),
        "[\"one\",\"two\"]"
    );
}

#[test]
fn text_content_takes_precedence_over_duplicate_raw_output() {
    let content = json!([
        {"type": "content", "content": {"type": "text", "text": "first"}},
        {"type": "content", "content": {"type": "text", "text": "second"}}
    ]);
    assert_eq!(
        tool_output_text(Some(&content), Some(&json!({"result": "duplicate"}))),
        "first\nsecond"
    );
    assert_eq!(
        tool_output_text(None, Some(&json!("line\n  indent"))),
        "line\n  indent"
    );
    assert_eq!(tool_output_text(None, Some(&json!(null))), "");
}
