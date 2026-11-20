use std::collections::VecDeque;

use gent_types::{NormalizedLifecycleSignal, ToolPhase};
use serde_json::{Value, json};

use super::{ClaurstAcpFact, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport};

#[derive(Default)]
struct Peer {
    reads: VecDeque<Vec<u8>>,
    writes: Vec<Value>,
    exit: Option<String>,
}

impl ClaurstAcpStdio for Peer {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(serde_json::from_slice(frame).unwrap());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(self.reads.pop_front())
    }
    fn exited(&mut self) -> Result<Option<String>, String> {
        Ok(self.exit.clone())
    }
}

fn read(transport: &mut ClaurstAcpTransport<Peer>, value: &Value) {
    transport
        .stdio
        .reads
        .push_back(serde_json::to_vec(value).unwrap());
}

fn update(transport: &mut ClaurstAcpTransport<Peer>, update: Value) {
    read(
        transport,
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":update}}),
    );
}

fn launch_ping(transport: &mut ClaurstAcpTransport<Peer>, prompt_id: u64, stop: &str) {
    transport.prompt("acp-1", "ping in the background").unwrap();
    update(
        transport,
        json!({"sessionUpdate":"tool_call","toolCallId":"toolu_ping","title":"Bash","kind":"execute","status":"in_progress","rawInput":{"command":"ping -n 40 127.0.0.1","run_in_background":true}}),
    );
    update(
        transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_ping","status":"in_progress","content":[{"type":"content","content":{"type":"text","text":"Command started in background."}}],"rawOutput":"Command started in background."}),
    );
    update(
        transport,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"started"}}),
    );
    read(
        transport,
        &json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":stop}}),
    );
}

fn transport() -> ClaurstAcpTransport<Peer> {
    let mut transport = ClaurstAcpTransport::new(Peer::default());
    transport.initialized = true;
    transport
}

fn tool_phases(facts: &[ClaurstAcpFact]) -> Vec<(String, String, ToolPhase)> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
                Some((
                    activity.tool_use_id.clone(),
                    activity.tool_name.clone(),
                    activity.phase.clone(),
                ))
            }
            _ => None,
        })
        .collect()
}

fn ping(phase: ToolPhase) -> (String, String, ToolPhase) {
    ("toolu_ping".into(), "Bash".into(), phase)
}

fn launched() -> ClaurstAcpTransport<Peer> {
    let mut transport = transport();
    launch_ping(&mut transport, 1, "end_turn");
    let drained = transport.drain(64).unwrap();
    assert_eq!(drained.terminal, Some(ClaurstAcpTerminal::Completed));
    assert_eq!(
        tool_phases(&drained.facts).last(),
        Some(&ping(ToolPhase::Background))
    );
    transport
}

#[test]
fn a_command_still_running_at_the_answer_becomes_background_and_keeps_the_runtime_live() {
    let mut transport = launched();
    assert!(transport.has_background_commands());
    assert!(!transport.is_idle());
    assert_eq!(
        transport
            .background_sessions()
            .into_iter()
            .collect::<Vec<_>>(),
        ["acp-1"]
    );
    transport.prompt("acp-1", "next").unwrap();
    assert!(transport.has_background_commands());
}

#[test]
fn a_background_command_end_is_drained_without_a_pending_prompt_and_keeps_its_name() {
    let mut transport = launched();
    assert!(transport.drain(64).unwrap().facts.is_empty());
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_ping","status":"completed","rawOutput":{"status":"completed"}}),
    );
    let drained = transport.drain(64).unwrap();
    assert_eq!(drained.terminal, None);
    assert_eq!(tool_phases(&drained.facts), [ping(ToolPhase::Completed)]);
    assert!(transport.is_idle());
}

#[test]
fn a_cancelled_background_command_ends_interrupted_and_a_failing_one_ends_failed() {
    for (raw_output, expected) in [
        (json!({"status":"cancelled"}), ToolPhase::Interrupted),
        (json!({"status":"failed: exit code 1"}), ToolPhase::Failed),
    ] {
        let mut transport = launched();
        update(
            &mut transport,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_ping","status":"failed","rawOutput":raw_output}),
        );
        let drained = transport.drain(64).unwrap();
        assert_eq!(tool_phases(&drained.facts), [ping(expected)]);
        assert!(!transport.has_background_commands());
    }
}

#[test]
fn a_command_whose_permission_the_answer_cancelled_is_not_background() {
    let mut transport = transport();
    transport.prompt("acp-1", "write").unwrap();
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":"toolu_write","title":"Write","kind":"edit","status":"in_progress","rawInput":{"file_path":"a.txt"}}),
    );
    read(
        &mut transport,
        &json!({"jsonrpc":"2.0","id":9,"method":"session/request_permission","params":{"sessionId":"acp-1","toolCall":{"toolCallId":"toolu_write","title":"Write","kind":"edit","rawInput":{"file_path":"a.txt"}},"options":[{"kind":"allow_once","optionId":"allow_once"}]}}),
    );
    update(
        &mut transport,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}}),
    );
    read(
        &mut transport,
        &json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}),
    );
    let mut facts = Vec::new();
    let mut terminal = None;
    for _ in 0..4 {
        let drained = transport.drain(64).unwrap();
        facts.extend(drained.facts);
        terminal = terminal.or(drained.terminal);
    }
    assert_eq!(terminal, Some(ClaurstAcpTerminal::Completed));
    assert!(
        transport
            .stdio
            .writes
            .iter()
            .any(|frame| frame["id"] == 9 && frame["result"]["outcome"]["outcome"] == "cancelled")
    );
    assert_eq!(
        tool_phases(&facts),
        [("toolu_write".into(), "Write".into(), ToolPhase::Started)]
    );
    assert!(!transport.has_background_commands());
}

#[test]
fn claurst_exiting_ends_its_background_commands_failed() {
    let mut transport = launched();
    transport.stdio.exit = Some("exit code 1".into());
    let drained = transport.drain(64).unwrap();
    assert_eq!(tool_phases(&drained.facts), [ping(ToolPhase::Failed)]);
    assert!(transport.is_idle());
}

#[test]
fn a_cancelled_answer_leaves_earlier_background_commands_to_the_runtime_release() {
    let mut transport = launched();
    transport.prompt("acp-1", "next").unwrap();
    transport.cancel("acp-1").unwrap();
    read(
        &mut transport,
        &json!({"jsonrpc":"2.0","id":2,"result":{"stopReason":"cancelled"}}),
    );
    let drained = transport.drain(64).unwrap();
    assert_eq!(drained.terminal, Some(ClaurstAcpTerminal::Interrupted));
    assert!(tool_phases(&drained.facts).is_empty());
    assert!(transport.has_background_commands());
}
