use std::{collections::VecDeque, path::Path};

use gent_types::NormalizedProviderEvent;

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
    let mut transport = ClaurstAcpTransport::new(fake);
    assert_eq!(
        transport
            .initialize_session(Path::new("/workspace"))
            .unwrap(),
        "acp-1"
    );
    transport.prompt("acp-1", "hi").unwrap();
    let drain = transport.drain(64).unwrap();
    assert_eq!(drain.terminal, Some(ClaurstAcpTerminal::Completed));
    assert_eq!(
        drain.facts,
        [ClaurstAcpFact::Event(NormalizedProviderEvent::Output {
            text: "hello".into(),
            is_partial: true
        })]
    );
}

#[test]
fn permission_request_is_denied_until_gent_permission_composition_exists() {
    let fake = Fake {
        writes: vec![],
        reads: VecDeque::from([
            frame(serde_json::json!({"id":1,"result":{}})),
            frame(serde_json::json!({"id":2,"result":{"sessionId":"acp-1"}})),
            frame(serde_json::json!({"id":99,"method":"session/request_permission","params":{}})),
        ]),
    };
    let mut transport = ClaurstAcpTransport::new(fake);
    transport
        .initialize_session(Path::new("/workspace"))
        .unwrap();
    let drain = transport.drain(1).unwrap();
    assert_eq!(drain.facts.len(), 1);
}
