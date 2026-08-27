use gent_drivers::{
    PublicProvider,
    public_protocol::{PublicWireFact, normalize_public_frame, replay_public_frames},
};
use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    RootActivity, ToolPhase, TurnPhase,
};
use serde_json::json;

#[test]
fn claude_stream_json_init_assistant_tool_and_result_are_normalized() {
    let facts = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"system","subtype":"init","session_id":"claude-1"}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"tool-1","name":"Read"}]}}),
            json!({"type":"result","is_error":false}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::SessionStarted {
        provider_session_id: "claude-1".into()
    }));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: false,
        }))
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready
        }
    )));
    assert!(facts.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_use_id == "tool-1" && activity.phase == ToolPhase::Started)));
}

#[test]
fn claude_nested_stream_events_preserve_text_thinking_and_tool_boundaries() {
    let facts = replay_public_frames(
        PublicProvider::Claude,
        &[
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"private","usage":{"input_tokens":4200,"context_window":16000}}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hel"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"consider"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"tool-1","name":"Read"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"x"}}}),
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Generating,
        }
    )));
    assert!(facts.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::ContextUsage {
            used_tokens: 4200,
            window_tokens: Some(16000),
        }
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hel".into(),
            is_partial: true,
        }))
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: "consider".into(),
            is_partial: true,
        }))
    );
    assert!(facts.iter().any(|fact| matches!(fact, PublicWireFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) if activity.tool_use_id == "tool-1" && activity.phase == ToolPhase::Started)));
    assert!(facts.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::ToolInputDelta {
            block_index: 1,
            partial_json: "{\"path\":\"x".into(),
        }
    )));
}

#[test]
fn claude_authoritative_assistant_snapshot_preserves_complete_thinking() {
    let facts = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"assistant","message":{"content":[
            {"type":"thinking","thinking":"complete reasoning"},
            {"type":"text","text":"answer"}
        ]}}),
    );

    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: "complete reasoning".into(),
            is_partial: false,
        }))
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "answer".into(),
            is_partial: false,
        }))
    );
}

#[test]
fn claude_user_tool_results_need_explicit_identity_and_name() {
    let completed = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"user","message":{"content":[{
            "type":"tool_result","tool_use_id":"tool-1","tool_name":"Read",
            "content":"private output"
        }]}}),
    );
    assert!(matches!(completed.as_slice(), [PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity { activity }
    )] if activity.tool_use_id == "tool-1" && activity.tool_name == "Read"
        && activity.phase == ToolPhase::Completed
        && activity.output_digest.as_deref().is_some_and(|digest| digest.starts_with("sha256:"))));
    assert!(!format!("{completed:?}").contains("private output"));

    assert_eq!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"user","message":{"content":[{
                "type":"tool_result","tool_use_id":"tool-1","content":"private output"
            }]}}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: "unresolvedClaudeToolResult".into()
            }
        )]
    );
}

#[test]
fn claude_background_launch_receipt_binds_provider_child_to_parent_without_output() {
    let facts = normalize_public_frame(
        PublicProvider::Claude,
        &json!({
            "type":"user",
            "message":{"content":[{
                "type":"tool_result",
                "tool_use_id":"parent-tool-1",
                "tool_name":"Task",
                "content":"Async agent launched successfully.\nagentId: child-1\noutput_file: /tmp/child-1.output"
            }]}
        }),
    );
    assert!(facts.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id,
            parent_tool_use_id
        }) if child_id == "child-1" && parent_tool_use_id == "parent-tool-1"
    )));
    assert!(!format!("{facts:?}").contains("/tmp/child-1.output"));
}

#[test]
fn claude_background_tasks_do_not_guess_a_tool_name() {
    assert_eq!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"system","subtype":"task_progress","tool_use_id":"parent-1"}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: "unresolvedClaudeBackgroundTask".into()
            }
        )]
    );
    let facts = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"system","subtype":"task_started","tool_use_id":"parent-1","tool_name":"Task"}),
    );
    assert!(matches!(facts.as_slice(), [PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity { activity }
    )] if activity.tool_use_id == "parent-1" && activity.tool_name == "Task"
        && activity.phase == ToolPhase::Started));
}

#[test]
fn claude_child_activity_is_not_flattened_into_the_root_transcript() {
    let facts = normalize_public_frame(
        PublicProvider::Claude,
        &json!({
            "type": "stream_event",
            "parent_tool_use_id": "parent-tool-1",
            "event": {"type": "subagent_activity", "kind": "text", "text": "private child output"}
        }),
    );
    assert_eq!(
        facts,
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: "unsupportedClaudeStreamEvent".into()
            }
        )]
    );
    assert!(!format!("{facts:?}").contains("private child output"));
}

#[test]
fn claude_control_and_telemetry_frames_are_reduced_without_protocol_drift() {
    assert!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"control_response","response":{"subtype":"success"}}),
        )
        .is_empty()
    );
    assert_eq!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"control_response","response":{"subtype":"error"}}),
        ),
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TransportDiagnostic {
                classification: "claudeControlResponseError".into(),
            }
        )]
    );
    assert!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"system","subtype":"thinking_tokens","tokens":4}),
        )
        .is_empty()
    );
    assert!(
        normalize_public_frame(
            PublicProvider::Claude,
            &json!({"type":"tool_progress","tool_name":"Read","tool_use_id":"tool-1"}),
        )
        .is_empty()
    );
}

#[test]
fn claude_errors_are_redacted_and_classified_without_poisoning_following_frames() {
    let auth = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"error","message":"401 unauthorized Bearer secret"}),
    );
    assert_eq!(
        auth,
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::Authentication,
                message: "Claude authentication failed.".into(),
            }
        )]
    );
    assert!(!format!("{auth:?}").contains("secret"));

    let nested = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"stream_event","event":{"type":"error","error":{"message":"context length exceeded"}}}),
    );
    assert_eq!(
        nested,
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: ProviderFailureClassification::ContextLimit,
                message: "Claude context limit reached.".into(),
            }
        )]
    );

    let result = normalize_public_frame(
        PublicProvider::Claude,
        &json!({"type":"result","is_error":true,"result":"rate limit reached"}),
    );
    assert!(result.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification: ProviderFailureClassification::RateLimited,
            message: "Claude rate limit reached.".into(),
        }
    )));
    assert!(result.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Failed
        }
    )));
}

#[test]
fn codex_app_server_turns_messages_tools_and_approvals_are_normalized() {
    let facts = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"method":"thread/started","params":{"thread":{"id":"thread-1"}}}),
            json!({"method":"turn/started","params":{"turn":{"id":"turn-1"}}}),
            json!({"method":"item/agentMessage/delta","params":{"delta":"hello"}}),
            json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"message-1","text":"hello"}}}),
            json!({"method":"item/started","params":{"item":{"type":"commandExecution","id":"tool-1"}}}),
            json!({"method":"item/commandExecution/requestApproval","params":{}}),
            json!({"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed"}}}),
        ],
    );
    assert!(facts.contains(&PublicWireFact::SessionStarted {
        provider_session_id: "thread-1".into()
    }));
    assert!(facts.contains(&PublicWireFact::Event(
        NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-1".into()
        }
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: false,
        }))
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: true,
        }))
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::AttentionRequired
    )));
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-1".into()
        }))
    );
}

#[test]
fn codex_flat_turn_completion_status_is_normalized_with_the_active_turn_id() {
    let facts = replay_public_frames(
        PublicProvider::Codex,
        &[
            json!({"method":"turn/started","params":{"turn":{"id":"turn-flat"}}}),
            json!({"method":"turn/completed","params":{"status":"completed","turnId":"turn-flat"}}),
        ],
    );
    assert!(
        facts.contains(&PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-flat".into()
        }))
    );
    assert!(facts.contains(&PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::RootPhase {
            phase: TurnPhase::Ready
        }
    )));
}

#[test]
fn codex_tool_output_deltas_preserve_streaming_output_without_raw_provider_frames() {
    for (method, field) in [
        ("item/commandExecution/outputDelta", "delta"),
        ("item/fileChange/outputDelta", "delta"),
        ("item/commandExecution/terminalInteraction", "stdin"),
    ] {
        let frame = json!({
            "method": method,
            "params": {"itemId": "tool-1", field: "tool output"}
        });
        assert_eq!(
            normalize_public_frame(PublicProvider::Codex, &frame),
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: "tool-1".into(),
                    text: "tool output".into(),
                    is_partial: true,
                }
            )]
        );
    }
    let patch = normalize_public_frame(
        PublicProvider::Codex,
        &json!({
            "method":"item/fileChange/patchUpdated",
            "params":{"itemId":"tool-1","changes":[{"path":"src/lib.rs","diff":"@@ -1 +1 @@\n-old\n+new"},{"path":"src/main.rs","patch":"@@ patch @@"}]}
        }),
    );
    assert_eq!(
        patch,
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "tool-1".into(),
                text: "@@ -1 +1 @@\n-old\n+new\n@@ patch @@".into(),
                is_partial: true,
            }
        )]
    );
    let mcp = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/mcpToolCall/progress","params":{"itemId":"tool-1","message":"connected"}}),
    );
    assert!(mcp.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { tool_use_id, text, is_partial })
            if tool_use_id == "tool-1" && text == "connected" && *is_partial
    )));
    let completed = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{"type":"mcpToolCall","id":"tool-1","result":{"content":[{"type":"text","text":"done"}]}}}}),
    );
    assert!(completed.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { tool_use_id, text, is_partial })
            if tool_use_id == "tool-1" && text.contains("done") && !*is_partial
    )));
    let command = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{"type":"commandExecution","id":"tool-1","aggregatedOutput":"done"}}}),
    );
    assert!(!command.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { .. })
    )));
    let file = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{"type":"fileChange","id":"tool-1","changes":[{"diff":"@@ patch @@"}]}}}),
    );
    assert!(file.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta { tool_use_id, text, is_partial })
            if tool_use_id == "tool-1" && text == "@@ patch @@" && !*is_partial
    )));
}

#[test]
fn codex_error_notifications_are_redacted_and_classified() {
    for (frame, classification, message) in [
        (
            json!({"method":"error","params":{"error":{"message":"Rate limit exceeded"}}}),
            ProviderFailureClassification::RateLimited,
            "Codex rate limit reached.",
        ),
        (
            json!({"method":"codex/event/stream_error","params":{"msg":{"message":"unauthorized access"}}}),
            ProviderFailureClassification::Authentication,
            "Codex authentication failed.",
        ),
        (
            json!({"method":"codex/event/stream_error","params":{"msg":{"message":"request failed","codex_error_info":"unauthorized"}}}),
            ProviderFailureClassification::Authentication,
            "Codex authentication failed.",
        ),
        (
            json!({"method":"codex/event/error","params":{"message":"context window exceeded"}}),
            ProviderFailureClassification::ContextLimit,
            "Codex context limit reached.",
        ),
    ] {
        assert_eq!(
            normalize_public_frame(PublicProvider::Codex, &frame),
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ProviderFailure {
                    classification,
                    message: message.into(),
                }
            )]
        );
    }
}

#[test]
fn codex_completed_subagent_launch_preserves_only_the_child_parent_relationship() {
    let facts = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{
            "type":"subAgentActivity",
            "kind":"started",
            "id":"parent-tool-1",
            "agentThreadId":"child-thread-1",
            "agentPath":"/private/task"
        }}}),
    );
    assert_eq!(
        facts,
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::ChildStarted {
                child_id: "child-thread-1".into(),
                parent_tool_use_id: "parent-tool-1".into(),
            }
        )]
    );
    assert!(!format!("{facts:?}").contains("/private/task"));
}

#[test]
fn codex_collab_launches_preserve_each_explicit_child_thread() {
    let facts = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/started","params":{"item":{
            "type":"collabAgentToolCall",
            "id":"parent-tool-1",
            "receiverThreadIds":["child-2","child-1"],
            "prompt":"private"
        }}}),
    );
    assert!(facts.iter().any(|fact| matches!(fact,
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id, parent_tool_use_id
        }) if child_id == "child-1" && parent_tool_use_id == "parent-tool-1")));
    assert!(facts.iter().any(|fact| matches!(fact,
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id, parent_tool_use_id
        }) if child_id == "child-2" && parent_tool_use_id == "parent-tool-1")));
    assert!(!format!("{facts:?}").contains("private"));
}

#[test]
fn codex_collab_state_maps_are_also_explicit_child_identity() {
    let facts = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/started","params":{"item":{
            "type":"collabAgentToolCall",
            "id":"parent-tool-1",
            "receiverThreadIds":[],
            "agentsStates":{"child-map-1":{"status":"running"}}
        }}}),
    );
    assert!(facts.iter().any(|fact| matches!(fact,
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
            child_id, parent_tool_use_id
        }) if child_id == "child-map-1" && parent_tool_use_id == "parent-tool-1")));
    let completed = normalize_public_frame(
        PublicProvider::Codex,
        &json!({"method":"item/completed","params":{"item":{
            "type":"collabAgentToolCall",
            "id":"parent-tool-1",
            "receiverThreadIds":["child-map-1"]
        }}}),
    );
    assert!(!completed.iter().any(|fact| matches!(
        fact,
        PublicWireFact::Event(NormalizedProviderEvent::ChildStarted { .. })
    )));
}

#[test]
fn malformed_or_unknown_provider_frames_are_safe_diagnostics() {
    for (provider, frame, classification) in [
        (
            PublicProvider::Claude,
            json!({"type":"system","subtype":"init"}),
            "malformedClaudeInit",
        ),
        (
            PublicProvider::Codex,
            json!({"method":"turn/completed","params":{"turn":{"id":"a","status":"future"}}}),
            "malformedCodexTurn",
        ),
    ] {
        assert_eq!(
            normalize_public_frame(provider, &frame),
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::TransportDiagnostic {
                    classification: classification.into()
                }
            )]
        );
    }
}
