use std::collections::VecDeque;

use gent_ports::ClaurstPermissionReply;
use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolPhase, WorkPhase};
use serde_json::{Value, json};

use crate::claurst_acp_transport::{
    ClaurstAcpFact, ClaurstAcpStdio, ClaurstAcpTerminal, ClaurstAcpTransport,
};

#[derive(Default)]
struct Peer {
    reads: VecDeque<Vec<u8>>,
    writes: Vec<Value>,
    ended: bool,
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
        Ok(self.ended.then(|| "exited".to_owned()))
    }
}

fn read(transport: &mut ClaurstAcpTransport<Peer>, value: Value) {
    transport
        .stdio
        .reads
        .push_back(serde_json::to_vec(&value).unwrap());
}

fn update(transport: &mut ClaurstAcpTransport<Peer>, update: Value) {
    read(
        transport,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"acp-1","update":update}}),
    );
}

const CHILD: &str = "toolu_agent";

fn child_waiting_after_its_parent_answered() -> ClaurstAcpTransport<Peer> {
    let mut transport = ClaurstAcpTransport::new(Peer::default());
    transport.initialized = true;
    transport
        .prompt("acp-1", "write notes in the background")
        .unwrap();
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":CHILD,"title":"Agent","status":"in_progress","rawInput":{"prompt":"write notes","run_in_background":true}}),
    );
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":CHILD,"status":"in_progress","rawOutput":"started"}),
    );
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":"toolu_write","title":"Write: notes.txt","kind":"edit","status":"in_progress","rawInput":{"file_path":"notes.txt","content":"hello"},"_meta":{"gentParentToolCallId":CHILD}}),
    );
    read(
        &mut transport,
        json!({"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"sessionId":"acp-1","toolCall":{"toolCallId":"toolu_write","title":"Write","kind":"edit","status":"pending","rawInput":{"file_path":"notes.txt","content":"hello"},"_meta":{"gentParentToolCallId":CHILD}},"options":[{"optionId":"allow_once","name":"Allow once","kind":"allow_once"},{"optionId":"reject_once","name":"Reject","kind":"reject_once"}]}}),
    );
    let asked = transport.drain(64).unwrap();
    assert_eq!(asked.permissions.len(), 1);
    assert_eq!(asked.permissions[0].child_id.as_deref(), Some(CHILD));
    assert_eq!(
        asked.facts.iter().find_map(|fact| match fact {
            ClaurstAcpFact::Event(NormalizedProviderEvent::ChildStarted { child_id, .. }) =>
                Some(child_id.as_str()),
            _ => None,
        }),
        Some(CHILD)
    );
    assert!(asked.facts.iter().any(|fact| matches!(
        fact,
        ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity })
            if activity.tool_use_id == "toolu_write"
                && activity.parent_tool_use_id.as_deref() == Some(CHILD)
    )));
    update(
        &mut transport,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"started"}}),
    );
    read(
        &mut transport,
        json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}),
    );
    let answered = transport.drain(64).unwrap();
    assert_eq!(answered.terminal, Some(ClaurstAcpTerminal::Completed));
    assert!(transport.stdio.writes.iter().all(|frame| frame["id"] != 7));
    transport
}

#[test]
fn a_childs_permission_stays_answerable_after_its_parent_turn_answered() {
    let mut transport = child_waiting_after_its_parent_answered();

    transport
        .respond_permission("claurst:n:7", ClaurstPermissionReply::AllowOnce)
        .unwrap();

    assert_eq!(
        transport.stdio.writes.last().unwrap()["result"]["outcome"]["optionId"],
        "allow_once"
    );
}

#[test]
fn stop_withdraws_a_childs_permission_and_ends_its_open_tools_with_the_child() {
    let mut transport = child_waiting_after_its_parent_answered();

    transport.cancel("acp-1").unwrap();
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":CHILD,"status":"failed","rawOutput":{"status":"cancelled"}}),
    );
    let ended = transport.drain(64).unwrap();

    assert!(
        transport
            .stdio
            .writes
            .iter()
            .any(|frame| frame["id"] == 7 && frame["result"]["outcome"]["outcome"] == "cancelled")
    );
    let order: Vec<_> = ended
        .facts
        .iter()
        .filter_map(|fact| match fact {
            ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
                Some(format!("{}:{:?}", activity.tool_use_id, activity.phase))
            }
            ClaurstAcpFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase }) => {
                assert_eq!(*phase, WorkPhase::Interrupted);
                Some(format!("child {child_id}"))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [
            format!("toolu_write:{:?}", ToolPhase::Interrupted),
            format!("child {CHILD}"),
            format!("{CHILD}:{:?}", ToolPhase::Interrupted),
        ]
    );
    assert!(!transport.has_background_commands());
}

fn tool_phases(facts: &[ClaurstAcpFact]) -> Vec<String> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            ClaurstAcpFact::Lifecycle(NormalizedLifecycleSignal::ToolActivity { activity }) => {
                Some(format!(
                    "{}:{:?}:{}",
                    activity.tool_use_id,
                    activity.phase,
                    activity.parent_tool_use_id.as_deref().unwrap_or("-")
                ))
            }
            ClaurstAcpFact::Event(NormalizedProviderEvent::ChildTerminal { child_id, phase }) => {
                Some(format!("child {child_id}:{phase:?}"))
            }
            _ => None,
        })
        .collect()
}

fn foreground_agent_with_children(children_report: bool) -> ClaurstAcpTransport<Peer> {
    let mut transport = ClaurstAcpTransport::new(Peer::default());
    transport.initialized = true;
    transport.prompt("acp-1", "delegate to one agent").unwrap();
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":CHILD,"title":"Agent","status":"in_progress","rawInput":{"prompt":"write child.txt then git status"}}),
    );
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":"toolu_write","title":"Write: child.txt","kind":"edit","status":"in_progress","rawInput":{"file_path":"child.txt","content":"hi"},"_meta":{"gentParentToolCallId":CHILD}}),
    );
    if children_report {
        update(
            &mut transport,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_write","status":"completed","rawOutput":"written"}),
        );
    }
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call","toolCallId":"toolu_bash","title":"Bash: git status --short","kind":"execute","status":"in_progress","rawInput":{"command":"git status --short","run_in_background":true},"_meta":{"gentParentToolCallId":CHILD}}),
    );
    if children_report {
        update(
            &mut transport,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_bash","status":"in_progress","rawOutput":"Command started in background."}),
        );
    }
    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":CHILD,"status":"completed","rawOutput":"child.txt written"}),
    );
    update(
        &mut transport,
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"The agent wrote child.txt."}}),
    );
    read(
        &mut transport,
        json!({"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}),
    );
    transport
}

#[test]
fn a_foreground_agents_children_end_with_their_own_reports_and_a_running_one_ends_when_it_finishes()
{
    let mut transport = foreground_agent_with_children(true);
    let answered = transport.drain(64).unwrap();

    assert_eq!(answered.terminal, Some(ClaurstAcpTerminal::Completed));
    assert_eq!(
        tool_phases(&answered.facts),
        [
            format!("{CHILD}:Started:-"),
            format!("toolu_write:Started:{CHILD}"),
            format!("toolu_write:Completed:{CHILD}"),
            format!("toolu_bash:Started:{CHILD}"),
            format!("toolu_bash:Started:{CHILD}"),
            format!("toolu_bash:Background:{CHILD}"),
            format!("child {CHILD}:Done"),
            format!("{CHILD}:Completed:-"),
        ]
    );
    assert!(transport.has_background_commands());

    update(
        &mut transport,
        json!({"sessionUpdate":"tool_call_update","toolCallId":"toolu_bash","status":"completed","rawOutput":"?? child.txt"}),
    );
    let finished = transport.drain(64).unwrap();

    assert_eq!(
        tool_phases(&finished.facts),
        [format!("toolu_bash:Completed:{CHILD}")]
    );
    assert!(!transport.has_background_commands());
}

#[test]
fn a_foreground_agents_unreported_children_end_once_when_the_runtime_ends() {
    let mut transport = foreground_agent_with_children(false);
    let answered = transport.drain(64).unwrap();
    assert_eq!(answered.terminal, Some(ClaurstAcpTerminal::Completed));

    transport.stdio.ended = true;
    let ended = transport.drain(64).unwrap();

    let phases: Vec<_> = tool_phases(&answered.facts)
        .into_iter()
        .chain(tool_phases(&ended.facts))
        .filter(|phase| phase.starts_with("toolu_write") || phase.starts_with("toolu_bash"))
        .collect();
    assert_eq!(
        phases,
        [
            format!("toolu_write:Started:{CHILD}"),
            format!("toolu_bash:Started:{CHILD}"),
            format!("toolu_write:Background:{CHILD}"),
            format!("toolu_bash:Background:{CHILD}"),
            format!("toolu_write:Failed:{CHILD}"),
            format!("toolu_bash:Failed:{CHILD}"),
        ]
    );
    assert!(!transport.has_background_commands());
}
