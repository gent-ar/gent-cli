use std::{collections::VecDeque, path::Path};

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};

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
fn overlapping_permission_requests_are_cancelled_with_the_nested_acp_outcome_shape() {
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
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    assert_eq!(transport.drain(1).unwrap().permissions.len(), 1);
    assert!(transport.drain(1).unwrap().permissions.is_empty());
    let response: serde_json::Value =
        serde_json::from_slice(transport.stdio.writes.last().unwrap()).unwrap();
    assert_eq!(response["id"], 100);
    assert_eq!(response["result"]["outcome"]["outcome"], "cancelled");
}

#[test]
fn projects_upstream_tool_call_lifecycle_without_retaining_raw_tool_data() {
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
                },
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
