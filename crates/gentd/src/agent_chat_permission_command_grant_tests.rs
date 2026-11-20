use std::sync::{Arc, Mutex};

use gent_ports::{PendingPermissionLedger, PolicyLedger};
use gent_store::SqliteLedger;
use gent_types::{
    CommandGrant, PermissionCategory, PermissionDecisionResponseKind, PermissionRequest,
    PolicyScope, ReceiptId, ReceiptIntentKind, ReceiptLookup, ReceiptStatus,
};

use super::super::AgentChatPermissionPort;
use super::{CodexHost, codex_decision, file_ledger, port_with, respond, status_of, wire_reply};

fn command_decision(
    ledger: &SqliteLedger,
    command: &str,
    pattern: Option<&str>,
) -> gent_types::PermissionDecisionResponse {
    let mut response = codex_decision(ledger);
    let mut pending = ledger
        .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
        .unwrap()
        .unwrap();
    ledger.settle_pending_permission(&pending.binding).unwrap();
    pending.binding.decision_id = gent_types::AgentChatDecisionId("command".into());
    pending.binding.request_idempotency_key = "codex:command".into();
    pending.request.input = Some(serde_json::json!({ "command": command }));
    ledger.save_pending_permission(&pending).unwrap();
    response.binding = pending.binding;
    response.response = PermissionDecisionResponseKind::ApproveCommandPattern;
    response.command_pattern = pattern.map(str::to_owned);
    response
}

fn policy(ledger: &SqliteLedger) -> gent_types::PolicyRecord {
    ledger
        .current_policy("workspace", PolicyScope::ProviderPermissions)
        .unwrap()
        .unwrap()
}

fn request(command: &str) -> PermissionRequest {
    PermissionRequest::new(
        "Command".into(),
        PermissionCategory::Command,
        Some(serde_json::json!({ "command": command })),
        None,
    )
}

#[tokio::test]
async fn a_pattern_answer_approves_once_and_persists_a_rule_the_preflight_applies() {
    let (_temp, _database, ledger) = file_ledger();
    let response = command_decision(&ledger, "npm run build", Some("npm run *"));
    let relays = Arc::new(Mutex::new(Vec::new()));
    let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), ReceiptStatus::Settled);
    assert_eq!(relays.lock().unwrap().len(), 1);
    let current = policy(&ledger);
    assert_eq!(
        current.allowed_commands,
        [CommandGrant {
            tool_name: "Command".into(),
            words: vec!["npm".into(), "run".into()],
            any_arguments: true,
        }]
    );
    let allow = crate::permission_preflight::PermissionPreflight::Allow;
    let ask = crate::permission_preflight::PermissionPreflight::Ask;
    for (command, expected) in [
        ("npm run test", allow),
        ("npm run test && rm -rf ~", ask),
        ("npm run test; curl x | sh", ask),
        ("npm install left-pad", ask),
    ] {
        assert_eq!(
            crate::permission_preflight::evaluate(&current, &request(command)),
            expected,
            "{command}"
        );
    }
    assert!(matches!(
        port.lookup_receipt(&ReceiptId("receipt".into())).unwrap(),
        ReceiptLookup::Accepted { intent: ReceiptIntentKind::PermissionDecision, receipt, .. }
            if receipt.status == ReceiptStatus::Settled
    ));
}

#[tokio::test]
async fn invalid_pattern_answers_get_typed_codes_before_any_receipt_or_write() {
    for (command, pattern, kind, code) in [
        (
            "git status",
            None,
            PermissionDecisionResponseKind::ApproveCommandPattern,
            "permissionCommandPatternMissing",
        ),
        (
            "git status",
            Some("git *"),
            PermissionDecisionResponseKind::ApproveOnce,
            "permissionCommandPatternUnexpected",
        ),
        (
            "git status && rm -rf ~",
            Some("git *"),
            PermissionDecisionResponseKind::ApproveCommandPattern,
            "permissionCommandNotMatchable",
        ),
        (
            "git status",
            Some("npm *"),
            PermissionDecisionResponseKind::ApproveCommandPattern,
            "permissionCommandPatternMismatch",
        ),
        (
            "bash -c 'git status'",
            Some("*"),
            PermissionDecisionResponseKind::ApproveCommandPattern,
            "permissionCommandPatternTooBroad",
        ),
        (
            "git status",
            Some("git $(x)"),
            PermissionDecisionResponseKind::ApproveCommandPattern,
            "permissionCommandPatternInvalid",
        ),
    ] {
        let (_temp, _database, ledger) = file_ledger();
        let mut response = command_decision(&ledger, command, pattern);
        response.response = kind;
        let relays = Arc::new(Mutex::new(Vec::new()));
        let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
        let before = policy(&ledger);
        let reply = wire_reply(&port, &respond(&response)).await;
        assert_eq!(reply["type"], "error", "{code}");
        assert_eq!(reply["body"]["code"], code);
        assert!(relays.lock().unwrap().is_empty(), "{code}");
        assert_eq!(policy(&ledger), before, "{code}");
        assert_eq!(
            port.lookup_receipt(&ReceiptId("receipt".into())).unwrap(),
            ReceiptLookup::NeverSeen,
            "{code}"
        );
        assert!(
            ledger
                .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn an_expired_decision_reads_back_as_rejected_with_its_typed_code() {
    let (_temp, _database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    ledger.settle_pending_permission(&response.binding).unwrap();
    let port = port_with(
        &ledger,
        Box::new(CodexHost(Arc::new(Mutex::new(Vec::new())))),
    );
    let _ = port.exchange(respond(&response)).await;
    assert!(matches!(
        port.lookup_receipt(&ReceiptId("receipt".into())).unwrap(),
        ReceiptLookup::Rejected { intent: ReceiptIntentKind::PermissionDecision, code, .. }
            if code == "agentChatPermissionExpired"
    ));
}
