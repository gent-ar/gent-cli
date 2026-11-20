use super::*;
use gent_types::ToolPhase;

const STARTED: &[u8] = br#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"exec-1","command":"ping -n 90 127.0.0.1","cwd":"/work","processId":"34657","source":"unifiedExecStartup","status":"inProgress","commandActions":[],"aggregatedOutput":null,"exitCode":null,"durationMs":null},"threadId":"thread-private","turnId":"turn-1"}}"#;
const ENDED: &[u8] = br#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"exec-1","command":"ping -n 90 127.0.0.1","cwd":"/work","processId":"34657","source":"unifiedExecStartup","status":"failed","commandActions":[],"aggregatedOutput":"","exitCode":null,"durationMs":1},"threadId":"thread-private","turnId":"turn-1"}}"#;
const TURN_ENDED: &[u8] = br#"{"method":"turn/completed","params":{"threadId":"thread-private","turn":{"id":"turn-1","items":[],"status":"interrupted","error":null}}}"#;

fn open_turn() -> CodexTurnDriver {
    let (mut driver, _) = CodexTurnDriver::start(config(), "hello", None).unwrap();
    driver.receive(br#"{"id":1,"result":{}}"#).unwrap();
    driver
        .receive(br#"{"id":2,"result":{"thread":{"id":"thread-private"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"method":"turn/started","params":{"threadId":"thread-private","turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
        .receive(br#"{"id":3,"result":{"turn":{"id":"turn-1"}}}"#)
        .unwrap();
    driver
}

fn phases(effects: &[CodexTurnEffect]) -> Vec<ToolPhase> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::Fact(PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity { activity },
            )) if activity.tool_use_id == "exec-1" => Some(activity.phase.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_command_still_running_when_its_turn_ends_is_reported_running_in_background() {
    let mut driver = open_turn();
    assert_eq!(
        phases(&driver.receive(STARTED).unwrap()),
        [ToolPhase::Started]
    );
    let turn_end = driver.receive(TURN_ENDED).unwrap();
    assert_eq!(phases(&turn_end), [ToolPhase::Background]);
    assert!(matches!(
        turn_end.first(),
        Some(CodexTurnEffect::Fact(PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::ToolActivity { .. }
        )))
    ));
    assert_eq!(phases(&driver.receive(ENDED).unwrap()), [ToolPhase::Failed]);
}

#[test]
fn stop_cleans_background_terminals_and_their_end_reads_interrupted() {
    let mut driver = open_turn();
    driver.receive(STARTED).unwrap();
    let CodexTurnEffect::Write(stop) = driver.interrupt().unwrap() else {
        panic!("Stop must write to Codex");
    };
    let methods: Vec<String> = stop
        .split_inclusive(|byte| *byte == b'\n')
        .map(|line| serde_json::from_slice::<Value>(line).unwrap()["method"].to_string())
        .collect();
    assert_eq!(
        methods,
        ["\"turn/interrupt\"", "\"thread/backgroundTerminals/clean\""]
    );
    assert!(driver.interrupt().is_err());
    assert_eq!(
        phases(&driver.receive(TURN_ENDED).unwrap()),
        [ToolPhase::Background]
    );
    assert!(
        driver
            .receive(br#"{"id":5,"result":{}}"#)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        phases(&driver.receive(ENDED).unwrap()),
        [ToolPhase::Interrupted]
    );
}

fn session_states(effects: &[CodexTurnEffect]) -> Vec<bool> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            CodexTurnEffect::SessionState { idle } => Some(*idle),
            _ => None,
        })
        .collect()
}

const APPROVAL: &[u8] = br#"{"id":9,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-private","turnId":"turn-1","itemId":"exec-1"}}"#;
const ROOT_IDLE: &[u8] = br#"{"method":"thread/status/changed","params":{"threadId":"thread-private","status":{"type":"idle"}}}"#;

#[test]
fn a_command_still_awaiting_approval_when_stop_ends_its_turn_is_interrupted_not_background() {
    let mut driver = open_turn();
    driver.receive(STARTED).unwrap();
    assert!(matches!(
        driver.receive(APPROVAL).unwrap().as_slice(),
        [CodexTurnEffect::ControlRequest(_)]
    ));
    driver.interrupt().unwrap();
    assert_eq!(
        phases(&driver.receive(TURN_ENDED).unwrap()),
        [ToolPhase::Interrupted]
    );
    assert_eq!(session_states(&driver.receive(ROOT_IDLE).unwrap()), [true]);
}

#[test]
fn an_approved_command_still_running_when_its_turn_ends_is_background() {
    let mut driver = open_turn();
    driver.receive(STARTED).unwrap();
    driver.receive(APPROVAL).unwrap();
    driver.approval_answered("exec-1");
    assert_eq!(
        phases(&driver.receive(TURN_ENDED).unwrap()),
        [ToolPhase::Background]
    );
    assert_eq!(session_states(&driver.receive(ROOT_IDLE).unwrap()), [false]);
}

#[test]
fn an_open_sub_agent_keeps_the_session_live_until_its_thread_ends() {
    let mut driver = open_turn();
    driver
        .receive(br#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","kind":"started","id":"parent-tool-1","agentThreadId":"child-thread-1"}}}"#)
        .unwrap();
    driver.receive(TURN_ENDED).unwrap();
    assert_eq!(session_states(&driver.receive(ROOT_IDLE).unwrap()), [false]);
    let approval = driver
        .receive(br#"{"id":11,"method":"item/commandExecution/requestApproval","params":{"threadId":"child-thread-1","turnId":"child-turn","itemId":"exec-9"}}"#)
        .unwrap();
    assert!(matches!(
        approval.as_slice(),
        [CodexTurnEffect::ControlRequest(request)]
            if request.child_id.as_deref() == Some("child-thread-1")
    ));
    let ended = driver
        .receive(br#"{"method":"turn/completed","params":{"threadId":"child-thread-1","turn":{"id":"child-turn","items":[],"status":"completed","error":null}}}"#)
        .unwrap();
    assert_eq!(session_states(&ended), [true]);
}

#[test]
fn a_running_background_command_keeps_the_session_live_until_it_ends() {
    let mut driver = open_turn();
    driver.receive(STARTED).unwrap();
    driver.receive(TURN_ENDED).unwrap();
    let idle = driver
        .receive(br#"{"method":"thread/status/changed","params":{"threadId":"thread-private","status":{"type":"idle"}}}"#)
        .unwrap();
    assert_eq!(session_states(&idle), [false]);
    let ended = driver.receive(ENDED).unwrap();
    assert_eq!(phases(&ended), [ToolPhase::Failed]);
    assert_eq!(session_states(&ended), [true]);
}
