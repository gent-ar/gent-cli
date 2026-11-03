use gent_drivers::codex_session::{CodexSessionConfig, CodexTurnOptions};
use gent_drivers::codex_turn::{CodexTurnDriver, CodexTurnEffect};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use serde_json::{Value, json};

fn driver() -> CodexTurnDriver {
    CodexTurnDriver::start(
        CodexSessionConfig {
            working_directory: None,
            resume_thread_id: None,
            turn_options: CodexTurnOptions::from_selection(
                &AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5.6".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
                None,
            )
            .unwrap(),
            mcp_servers: None,
        },
        "hello",
        None,
    )
    .unwrap()
    .0
}

fn receive(driver: &mut CodexTurnDriver, frame: Value) -> Vec<CodexTurnEffect> {
    driver
        .receive(&serde_json::to_vec(&frame).unwrap())
        .unwrap()
}

fn started(thread: &str, turn: &str, item: &str, changes: Value) -> Value {
    json!({"method":"item/started","params":{"threadId":thread,"turnId":turn,
        "item":{"id":item,"type":"fileChange","changes":changes}}})
}

fn approval(thread: &str, turn: &str, item: &str) -> Value {
    json!({"id":0,"method":"item/fileChange/requestApproval","params":{
        "threadId":thread,"turnId":turn,"itemId":item,"reason":null,"grantRoot":null}})
}

fn input(driver: &mut CodexTurnDriver, frame: Value) -> Value {
    let effects = receive(driver, frame);
    let [CodexTurnEffect::ControlRequest(request)] = effects.as_slice() else {
        panic!("expected approval request")
    };
    request.input.clone().unwrap()
}

fn changes(path: &str, diff: &str) -> Value {
    json!([{"path":path,"diff":diff,"kind":{"type":"add"}}])
}

#[test]
fn file_approval_retains_the_exact_started_item_operation_before_ledger_commit() {
    let mut driver = driver();
    let change = changes(
        "X:/fixture/codex-permission-probe.txt",
        "GENT-PARITY-CODEX-PERMISSION-OK\n",
    );
    receive(
        &mut driver,
        started("thread-1", "turn-1", "tool-1", change.clone()),
    );
    let input = input(&mut driver, approval("thread-1", "turn-1", "tool-1"));
    assert_eq!(input["changes"], change);
    assert_eq!(input["itemId"], "tool-1");
    assert!(input["grantRoot"].is_null() && input["reason"].is_null());
    let activity = gent_types::operation_preview(&input).unwrap();
    assert_eq!(activity["changes"], change);
}

#[test]
fn explicit_native_approval_changes_override_earlier_item_preview() {
    let mut driver = driver();
    receive(
        &mut driver,
        started("thread-1", "turn-1", "tool-1", changes("old.txt", "old")),
    );
    let mut request = approval("thread-1", "turn-1", "tool-1");
    request["params"]["changes"] = changes("new.txt", "new");
    request["params"]["availableDecisions"] = json!(["accept", "decline"]);
    let input = input(&mut driver, request);
    assert_eq!(input["changes"], changes("new.txt", "new"));
    assert_eq!(input["availableDecisions"], json!(["accept", "decline"]));
}

#[test]
fn unrelated_thread_turn_or_item_cannot_supply_approval_details() {
    let mut driver = driver();
    receive(
        &mut driver,
        started(
            "thread-1",
            "turn-1",
            "tool-1",
            changes("owned.txt", "owned"),
        ),
    );
    for (thread, turn, item) in [
        ("thread-2", "turn-1", "tool-1"),
        ("thread-1", "turn-2", "tool-1"),
        ("thread-1", "turn-1", "tool-2"),
    ] {
        assert!(
            input(&mut driver, approval(thread, turn, item))
                .get("changes")
                .is_none()
        );
    }
    let mut incomplete = approval("thread-1", "turn-1", "tool-1");
    incomplete["params"]
        .as_object_mut()
        .unwrap()
        .remove("turnId");
    assert!(input(&mut driver, incomplete).get("changes").is_none());
}

#[test]
fn completed_items_release_but_new_turns_preserve_late_operation_correlation() {
    let mut driver = driver();
    receive(
        &mut driver,
        started(
            "thread-1",
            "turn-1",
            "tool-1",
            changes("owned.txt", "owned"),
        ),
    );
    receive(
        &mut driver,
        json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"tool-1","type":"fileChange","status":"completed"}}}),
    );
    assert!(
        input(&mut driver, approval("thread-1", "turn-1", "tool-1"))
            .get("changes")
            .is_none()
    );
    receive(
        &mut driver,
        started(
            "thread-1",
            "turn-1",
            "tool-2",
            changes("other.txt", "owned"),
        ),
    );
    receive(
        &mut driver,
        json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-2"}}}),
    );
    assert_eq!(
        input(&mut driver, approval("thread-1", "turn-1", "tool-2"))["changes"],
        changes("other.txt", "owned")
    );
    assert!(
        input(&mut driver, approval("thread-1", "turn-2", "tool-2"))
            .get("changes")
            .is_none()
    );
}

#[test]
fn correlated_preview_remains_redacted_utf8_safe_and_within_approval_budget() {
    let mut driver = driver();
    let changes = json!([{"path":"X:/fixture/界.txt","diff":"界\n".repeat(100_000),"authorization":"Bearer private"}]);
    receive(
        &mut driver,
        started("thread-1", "turn-1", "tool-1", changes),
    );
    let mut request = approval("thread-1", "turn-1", "tool-1");
    request["params"]["reason"] = json!("r".repeat(8000));
    let input = input(&mut driver, request);
    assert_eq!(input["changes"][0]["path"], "X:/fixture/界.txt");
    assert!(!input.to_string().contains("private"));
    assert!(input.to_string().contains("[truncated]"));
    assert!(serde_json::to_vec(&input).unwrap().len() <= 16 * 1024);
}

#[test]
fn command_approval_uses_the_same_normalized_operation_correlation() {
    let mut driver = driver();
    receive(
        &mut driver,
        json!({"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"tool-1","type":"commandExecution","command":"echo safe","cwd":"X:/fixture"}}}),
    );
    let request = json!({"id":1,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"tool-1"}});
    let input = input(&mut driver, request);
    assert_eq!(input["command"], "echo safe");
    assert_eq!(input["cwd"], "X:/fixture");
}

#[test]
fn multiple_files_and_rename_targets_survive_the_shared_approval_preview() {
    let mut driver = driver();
    let mut change = json!([
        {"path":"before.txt","kind":{"type":"update","move_path":"after.txt"},"diff":"body"},
        {"path":"created.txt","kind":{"type":"add"},"diff":"new"}
    ]);
    receive(
        &mut driver,
        started("thread-1", "turn-1", "tool-1", change.clone()),
    );
    let preview = input(&mut driver, approval("thread-1", "turn-1", "tool-1"));
    assert_eq!(preview["changes"], change);
    change[0]["diff"] = json!("large body\n".repeat(5000));
    receive(&mut driver, started("thread-1", "turn-1", "tool-2", change));
    let preview = input(&mut driver, approval("thread-1", "turn-1", "tool-2"));
    assert_eq!(preview["changes"][0]["path"], "before.txt");
    assert_eq!(preview["changes"][0]["kind"]["move_path"], "after.txt");
    assert!(preview.to_string().contains("[truncated]"));
}

#[test]
fn operation_cache_evicts_only_at_its_finite_recent_item_bound() {
    let mut driver = driver();
    for index in 0..=256 {
        receive(
            &mut driver,
            started(
                "thread-1",
                "turn-1",
                &format!("tool-{index}"),
                changes("owned.txt", "owned"),
            ),
        );
    }
    assert!(
        input(&mut driver, approval("thread-1", "turn-1", "tool-0"))
            .get("changes")
            .is_none()
    );
    assert_eq!(
        input(&mut driver, approval("thread-1", "turn-1", "tool-256"))["changes"],
        changes("owned.txt", "owned")
    );
}
