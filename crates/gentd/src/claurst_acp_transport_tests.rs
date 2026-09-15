use std::{collections::VecDeque, path::Path, time::Duration};

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, PermissionCategory, ToolActivity, ToolPhase,
};

use gent_ports::ClaurstPermissionReply;

use super::{ClaurstAcpFact, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport};

struct Fake {
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
}

impl ClaurstAcpStdio for Fake {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(frame.to_vec());
        Ok(())
    }

    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

fn frame(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn packaged_cold_start_has_the_bounded_acp_handshake_budget() {
    assert_eq!(
        super::HANDSHAKE_TIMEOUT,
        crate::provider_launch_budget::FIRST_EXECUTION_ALLOWANCE + Duration::from_secs(20)
    );
}

#[test]
fn frames_upstream_handshake_prompt_stream_and_terminal_without_blocking() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}})),
            frame(serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}),
            ),
            frame(serde_json::json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake).with_mcp_servers(vec![serde_json::json!({
        "name": "filesystem",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem"]
    })]);
    assert_eq!(
        transport
            .initialize_session(Path::new("/workspace"))
            .unwrap(),
        "acp-1"
    );
    let initialize: serde_json::Value = serde_json::from_slice(&transport.stdio.writes[1]).unwrap();
    assert_eq!(initialize["params"]["mcpServers"][0]["name"], "filesystem");
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Completed));
    assert_eq!(
        drain.facts,
        [
            ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
                text: "hello".into(),
                is_partial: true
            }),
            ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
                text: "hello".into(),
                is_partial: false
            }),
        ]
    );
}

#[test]
fn retains_the_exact_prompt_error_before_terminal_failure() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(serde_json::json!({"id":3,"error":{"code":-32602,"message":"unknown model"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Failed));
    assert_eq!(
        drain.facts,
        [ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text: "Claurst ACP prompt failed: {\"code\":-32602,\"message\":\"unknown model\"}"
                .into(),
            is_partial: false,
        })]
    );
}

#[test]
fn output_limit_is_an_explicit_failure_instead_of_a_false_completion() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(serde_json::json!({"id":3,"result":{"stopReason":"max_tokens"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Failed));
    assert_eq!(
        drain.facts,
        [ClaurstAcpFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: gent_types::ProviderFailureClassification::OutputLimit,
                message: "Claurst exhausted its output limit before completing the turn.".into(),
            }
        )]
    );
}

#[test]
fn claurst_no_response_placeholder_settles_as_a_typed_output_limit_failure() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"(no response — model ended the turn with stop_reason \"max_tokens\")"}}}}),
            ),
            frame(serde_json::json!({"id":3,"result":{"stopReason":"end_turn"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Failed));
    assert_eq!(
        drain.facts,
        [ClaurstAcpFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: gent_types::ProviderFailureClassification::OutputLimit,
                message: "Claurst exhausted its output limit before completing the turn.".into(),
            }
        )]
    );
}

#[test]
fn thinking_only_end_turn_is_an_explicit_failure() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}}}),
            ),
            frame(serde_json::json!({"id":3,"result":{"stopReason":"end_turn"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Failed));
    assert_eq!(
        drain.facts.last(),
        Some(&ClaurstAcpFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: gent_types::ProviderFailureClassification::Provider,
                message: "Claurst ended the turn without an assistant response.".into(),
            }
        ))
    );
}

#[test]
fn permission_request_is_held_then_relays_only_a_closed_gent_reply() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"id":99,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"tool-1","title":"Bash: rm -rf /","kind":"execute","rawInput":{"command":"rm -rf /"}}}}),
            ),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let drain = transport.drain(1).unwrap();
    assert_eq!(drain.facts.len(), 0);
    assert_eq!(drain.permissions.len(), 1);
    assert_eq!(drain.permissions[0].request_id, "99");
    assert_eq!(drain.permissions[0].tool_use_id, "tool-1");
    assert_eq!(drain.permissions[0].tool_name, "Bash");
    assert!(transport.stdio.writes.iter().all(|frame| {
        !frame
            .windows(b"rm -rf /".len())
            .any(|window| window == b"rm -rf /")
    }));
    transport
        .respond_permission("99", ClaurstPermissionReply::AllowOnce)
        .unwrap();
    let response: serde_json::Value =
        serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
    assert_eq!(response["id"], 99);
    assert_eq!(response["result"]["outcome"]["outcome"], "selected");
    assert_eq!(response["result"]["outcome"]["optionId"], "allow_once");
}

#[test]
fn a_claurst_permission_names_the_tool_call_it_gates_with_its_input_and_category() {
    let update = |update: serde_json::Value| {
        frame(
            serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":update}}),
        )
    };
    let permission = |id: u64, title: &str, kind: &str| {
        frame(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/request_permission",
            "params": {"sessionId": "acp-1", "toolCall": {"toolCallId": format!("perm-acp-1-{id}"), "title": title, "kind": kind, "status": "pending"}}
        }))
    };
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            update(
                serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"read-1","title":"Read: /workspace/README.md","kind":"read","status":"in_progress","rawInput":{"file_path":"/workspace/README.md"}}),
            ),
            update(
                serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"bash-1","title":"Bash: cargo test","kind":"execute","status":"in_progress","rawInput":{"command":"cargo test"}}),
            ),
            update(
                serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"glob-1","title":"Glob: **/*","kind":"search","status":"in_progress","rawInput":{"pattern":"**/*"}}),
            ),
            permission(7, "Runs the project test suite", "execute"),
            permission(8, "Tool 'Glob' requires approval", "read"),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let mut permissions = Vec::new();
    for _ in 0..4 {
        let drain = transport.drain(8).unwrap();
        if let Some(permission) = drain.permissions.into_iter().next() {
            transport
                .respond_permission(&permission.request_id, ClaurstPermissionReply::AllowOnce)
                .unwrap();
            permissions.push(permission);
        }
    }
    let summary = permissions
        .iter()
        .map(|permission| {
            (
                permission.tool_use_id.as_str(),
                permission.tool_name.as_str(),
                permission.category,
                permission.input.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (
                "bash-1",
                "Bash",
                PermissionCategory::Command,
                Some(serde_json::json!({"command": "cargo test"}))
            ),
            (
                "glob-1",
                "Glob",
                PermissionCategory::Read,
                Some(serde_json::json!({"pattern": "**/*"}))
            ),
        ]
    );
}

#[test]
fn permission_category_follows_the_acp_tool_kind() {
    let request = |id: u64, kind: &str| {
        frame(serde_json::json!({
            "id": id,
            "method": "session/request_permission",
            "params": {"toolCall": {"toolCallId": format!("perm-{id}"), "title": "Tool 'Tool' requires approval", "kind": kind, "status": "pending"}}
        }))
    };
    let kinds = [
        ("read", PermissionCategory::Read),
        ("search", PermissionCategory::Read),
        ("edit", PermissionCategory::Edit),
        ("delete", PermissionCategory::Edit),
        ("move", PermissionCategory::Edit),
        ("execute", PermissionCategory::Command),
        ("fetch", PermissionCategory::Network),
        ("other", PermissionCategory::Provider),
    ];
    for (index, (kind, category)) in kinds.into_iter().enumerate() {
        let id = 90 + index as u64;
        let fake = Fake {
            writes: vec![],
            reads: VecDeque::from([
                frame(serde_json::json!({"id":1,"result":{}})),
                frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
                request(id, kind),
            ]),
        };
        let mut transport = ClaurstAcpTransport::new(fake);
        transport
            .initialize_session(Path::new("/workspace"))
            .unwrap();
        let drain = transport.drain(1).unwrap();
        assert_eq!(drain.permissions[0].category, category, "{kind}");
    }
}

#[test]
fn permission_denial_uses_the_nested_acp_outcome_shape() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"id":99,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"tool-1","title":"Bash: pwd","kind":"execute"}}}),
            ),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    assert_eq!(transport.drain(1).unwrap().permissions.len(), 1);
    transport
        .respond_permission("99", ClaurstPermissionReply::Deny)
        .unwrap();
    let response: serde_json::Value =
        serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
    assert_eq!(response["id"], 99);
    assert_eq!(response["result"]["outcome"]["outcome"], "cancelled");
}

#[test]
fn overlapping_permission_requests_are_held_and_presented_in_order() {
    let permission = |id, tool_id| {
        frame(serde_json::json!({
            "id": id,
            "method": "session/request_permission",
            "params": {"toolCall": {"toolCallId": tool_id, "title": "Bash: pwd", "kind": "execute"}}
        }))
    };
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            permission(99, "tool-1"),
            permission(100, "tool-2"),
            permission(101, "tool-3"),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let writes_before = transport.stdio.writes.len();
    let first = transport.drain(8).unwrap().permissions;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].request_id, "99");
    assert!(transport.drain(8).unwrap().permissions.is_empty());
    assert_eq!(transport.stdio.writes.len(), writes_before);
    assert_eq!(
        transport.respond_permission("100", ClaurstPermissionReply::AllowOnce),
        Err(super::ClaurstAcpTransportError::InvalidPermission)
    );

    let mut answered = Vec::new();
    for (expected, reply) in [
        ("99", ClaurstPermissionReply::AllowOnce),
        ("100", ClaurstPermissionReply::Deny),
        ("101", ClaurstPermissionReply::AllowOnce),
    ] {
        let permissions = if expected == "99" {
            first.clone()
        } else {
            transport.drain(8).unwrap().permissions
        };
        assert_eq!(permissions.len(), 1);
        assert_eq!(permissions[0].request_id, expected);
        transport.respond_permission(expected, reply).unwrap();
        let response: serde_json::Value =
            serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
        answered.push((
            response["id"].clone(),
            response["result"]["outcome"]["outcome"].clone(),
        ));
    }
    assert_eq!(
        answered,
        [
            (serde_json::json!(99), serde_json::json!("selected")),
            (serde_json::json!(100), serde_json::json!("cancelled")),
            (serde_json::json!(101), serde_json::json!("selected")),
        ]
    );
    assert!(transport.drain(8).unwrap().permissions.is_empty());
}

#[test]
fn cancelling_a_turn_settles_every_held_permission_request() {
    let permission = |id, tool_id| {
        frame(serde_json::json!({
            "id": id,
            "method": "session/request_permission",
            "params": {"toolCall": {"toolCallId": tool_id, "title": "Bash: pwd", "kind": "execute"}}
        }))
    };
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            permission(99, "tool-1"),
            permission(100, "tool-2"),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    let session = transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    assert_eq!(transport.drain(8).unwrap().permissions.len(), 1);
    assert!(transport.drain(8).unwrap().permissions.is_empty());
    transport.cancel(&session).unwrap();
    let cancelled = transport
        .stdio
        .writes
        .iter()
        .map(|frame| serde_json::from_slice::<serde_json::Value>(frame).unwrap())
        .filter(|frame| frame["result"]["outcome"]["outcome"] == "cancelled")
        .map(|frame| frame["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(cancelled, [serde_json::json!(99), serde_json::json!(100)]);
    assert!(transport.drain(8).unwrap().permissions.is_empty());
}

#[test]
fn projects_upstream_tool_call_lifecycle_and_keeps_its_output_for_later_providers() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Bash: pwd","kind":"execute","status":"in_progress","rawInput":{"command":"pwd"}}}}),
            ),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tool-1","fields":{"status":"completed","rawOutput":"/workspace"}}}}),
            ),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let drain = transport.drain(8).unwrap();
    assert_eq!(
        drain.facts,
        [
            ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: "tool-1".into(),
                    tool_name: "Bash".into(),
                    phase: ToolPhase::Started,
                    output_digest: None,
                    parent_tool_use_id: None,
                },
            }),
            ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: "tool-1".into(),
                    tool_name: "Bash".into(),
                    phase: ToolPhase::Completed,
                    output_digest: Some(
                        "sha256:4384b4849b7f004db243393653c15565c0a2ab2a8951d8513646c37d2a14a51f"
                            .into(),
                    ),
                    parent_tool_use_id: None,
                },
            }),
            ClaurstAcpFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "tool-1".into(),
                text: "/workspace".into(),
                is_partial: false,
            }),
        ]
    );
}

#[test]
fn claurst_flattened_tool_call_update_carries_its_content_text() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Read: vault.txt","kind":"read","status":"in_progress"}}}),
            ),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"1\tThe vault token is V-1."}}],"rawOutput":"1\tThe vault token is V-1."}}}),
            ),
            frame(serde_json::json!({"jsonrpc":"2.0","method":super::OVERSIZED_FRAME_METHOD})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let facts = transport.drain(8).unwrap().facts;
    assert!(matches!(
        &facts[1],
        ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity })
            if activity.phase == ToolPhase::Completed && activity.tool_name == "Read"
    ));
    assert_eq!(
        facts[2..],
        [
            ClaurstAcpFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: "tool-1".into(),
                text: "1\tThe vault token is V-1.".into(),
                is_partial: false,
            }),
            ClaurstAcpFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification: gent_types::OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC.into(),
            }),
        ]
    );
}

#[test]
fn textual_tool_call_content_does_not_satisfy_structured_tool_call_gate() {
    let textual_call = r#"<tool_call>{"name":"get_marker","arguments":{}}</tool_call>"#;
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(
                serde_json::json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":textual_call}}}}),
            ),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let drain = transport.drain(8).unwrap();
    assert!(drain.permissions.is_empty());
    assert_eq!(
        drain.facts,
        [ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text: "<tool_call>{\"name\":\"get_marker\",\"arguments\":{}}</tool_call>".into(),
            is_partial: true,
        })]
    );
    assert!(!drain.facts.iter().any(|fact| matches!(
        fact,
        ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { .. })
    )));
}

#[test]
fn retains_the_negotiated_image_capability_for_prompt_delivery() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(
                serde_json::json!({"id":1,"result":{"agentCapabilities":{"promptCapabilities":{"image":true}}}}),
            ),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    assert!(transport.supports_images());
    transport
        .prompt_content(
            "acp-1",
            vec![serde_json::json!({"type":"image","data":"YWJj","mimeType":"image/png"})],
        )
        .unwrap();
    let prompt: serde_json::Value =
        serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
    assert_eq!(prompt["params"]["prompt"][0]["type"], "image");
}

#[test]
fn cancels_only_the_requested_session_without_inventing_a_prompt_terminal() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    transport.cancel("acp-1").unwrap();
    let cancel: serde_json::Value =
        serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
    assert_eq!(cancel["method"], "session/cancel");
    assert_eq!(cancel["params"]["sessionId"], "acp-1");
    assert!(cancel.get("id").is_none());
    assert_eq!(transport.drain(1).unwrap().terminal, None);
}
