use super::{ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport, ClaurstAcpTransportError};
use serde_json::{Value, json};
use std::collections::VecDeque;

#[derive(Default)]
struct Peer {
    reads: VecDeque<Vec<u8>>,
    writes: Vec<Value>,
}

impl ClaurstAcpStdio for Peer {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(serde_json::from_slice(frame).unwrap());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
}

fn prompting() -> ClaurstAcpTransport<Peer> {
    let mut transport = ClaurstAcpTransport::new(Peer::default());
    transport.initialized = true;
    transport
        .prompt("acp-1", "start a background agent")
        .unwrap();
    transport
}

fn read(transport: &mut ClaurstAcpTransport<Peer>, value: Value) {
    transport
        .stdio
        .reads
        .push_back(serde_json::to_vec(&value).unwrap());
}

fn permission(transport: &mut ClaurstAcpTransport<Peer>, id: u64, tool: &str) {
    read(
        transport,
        json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{"sessionId":"acp-1","toolCall":{"toolCallId":tool,"title":"Write","kind":"edit","rawInput":{"file_path":"animals.txt"}},"options":[{"kind":"allow_once","optionId":"allow_once"},{"kind":"reject_once","optionId":"reject_once"}]}}),
    );
}

fn answer(transport: &mut ClaurstAcpTransport<Peer>, outcome: &str, value: Value) {
    read(
        transport,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"started"}}}}),
    );
    let mut frame = json!({"jsonrpc":"2.0","id":1});
    frame[outcome] = value;
    read(transport, frame);
}

fn cancelled_answers(transport: &ClaurstAcpTransport<Peer>) -> Vec<u64> {
    transport
        .stdio
        .writes
        .iter()
        .filter(|frame| frame.pointer("/result/outcome/outcome") == Some(&json!("cancelled")))
        .filter_map(|frame| frame["id"].as_u64())
        .collect()
}

fn terminal(transport: &mut ClaurstAcpTransport<Peer>) -> Option<ClaurstAcpTerminal> {
    (0..8).find_map(|_| transport.drain(64).unwrap().terminal)
}

#[test]
fn a_turn_that_ends_with_a_permission_still_open_answers_it_and_takes_the_next_prompt() {
    let mut transport = prompting();
    permission(&mut transport, 7, "background-write");
    assert_eq!(transport.drain(64).unwrap().permissions.len(), 1);
    answer(&mut transport, "result", json!({"stopReason":"end_turn"}));

    assert_eq!(
        terminal(&mut transport),
        Some(ClaurstAcpTerminal::Completed)
    );
    assert_eq!(cancelled_answers(&transport), [7]);
    assert!(transport.is_idle());
    assert_eq!(transport.prompt("acp-1", "next"), Ok(()));
}

#[test]
fn every_held_permission_is_answered_when_its_prompt_fails() {
    let mut transport = prompting();
    permission(&mut transport, 7, "background-write");
    permission(&mut transport, 8, "background-edit");
    assert_eq!(transport.drain(64).unwrap().permissions.len(), 1);
    answer(
        &mut transport,
        "error",
        json!({"code":-32603,"message":"model stopped"}),
    );

    assert_eq!(terminal(&mut transport), Some(ClaurstAcpTerminal::Failed));
    assert_eq!(cancelled_answers(&transport), [7, 8]);
    assert!(transport.drain(64).unwrap().permissions.is_empty());
    assert_eq!(transport.prompt("acp-1", "next"), Ok(()));
}

#[test]
fn a_cancelled_turn_answers_its_permissions_once() {
    let mut transport = prompting();
    permission(&mut transport, 7, "background-write");
    assert_eq!(transport.drain(64).unwrap().permissions.len(), 1);
    transport.cancel("acp-1").unwrap();
    answer(&mut transport, "result", json!({"stopReason":"cancelled"}));

    assert_eq!(
        terminal(&mut transport),
        Some(ClaurstAcpTerminal::Interrupted)
    );
    assert_eq!(cancelled_answers(&transport), [7]);
    assert_eq!(transport.prompt("acp-1", "next"), Ok(()));
}

#[test]
fn a_prompt_while_another_is_unanswered_is_refused_as_in_flight() {
    let mut transport = prompting();
    assert_eq!(
        transport.prompt("acp-1", "next"),
        Err(ClaurstAcpTransportError::PromptInFlight)
    );
}
