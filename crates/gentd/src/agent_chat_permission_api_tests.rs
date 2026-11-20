use std::sync::{Arc, Mutex};

use gent_ports::{
    AgentChatReadLedger, AgentChatSelectionLedger, AgentChatWorkspaceLedger,
    ConversationActivityLedger, PendingPermissionLedger, PolicyLedger,
};
use gent_protocol::{AgentChatPermissionFrame, read_json_frame};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatDecisionId, AgentChatEffort,
    AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch,
    ContextPolicy, HostEpoch, PermissionCategory, PermissionDecisionBinding,
    PermissionDecisionRequest, PermissionDecisionResponse, PermissionDecisionResponseKind,
    PermissionRequest, PermissionRequestDigest, ReceiptId, WorkspaceRecord,
};
use tokio::io::duplex;

use super::{StandaloneAgentChatPermissionPort, receipt::permission_decision_accepted_event_id};
use crate::{
    agent_chat_permission_transport::dispatch_port,
    ordinary_lifecycle_cadence::pair_with_standalone_readiness,
    ordinary_lifecycle_host::Undelivered,
    ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter},
};

fn pending(
    binding: &PermissionDecisionBinding,
    tool_name: &str,
    input: Option<serde_json::Value>,
) -> PermissionDecisionRequest {
    PermissionDecisionRequest {
        binding: binding.clone(),
        request: PermissionRequest {
            tool_name: tool_name.into(),
            category: PermissionCategory::Command,
            input,
            child_id: None,
        },
    }
}

type CodexResponses = Arc<Mutex<Vec<(String, String, Option<serde_json::Value>)>>>;

struct CodexHost(CodexResponses);

impl OrdinaryLifecycleHost for CodexHost {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn wake(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn drive(&mut self) -> Result<(), crate::ordinary_lifecycle_host::DriveFailure> {
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        false
    }
    fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        _: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), Undelivered> {
        self.0
            .lock()
            .unwrap()
            .push((run_id.into(), request_id.into(), answers));
        Ok(())
    }
}

struct FailingCodexHost {
    writes: Arc<Mutex<usize>>,
    failure: Undelivered,
    interrupts: Arc<Mutex<Vec<String>>>,
}
impl OrdinaryLifecycleHost for FailingCodexHost {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn wake(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn drive(&mut self) -> Result<(), crate::ordinary_lifecycle_host::DriveFailure> {
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        false
    }
    fn respond_codex_permission(
        &self,
        _: &str,
        _: &str,
        _: gent_drivers::codex_control::CodexControlDecision,
        _: Option<serde_json::Value>,
    ) -> Result<(), Undelivered> {
        *self.writes.lock().unwrap() += 1;
        Err(self.failure)
    }
    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.interrupts.lock().unwrap().push(run_id.into());
        Ok(())
    }
}

struct ReceiptFailureCodexHost {
    database: std::path::PathBuf,
    writes: Arc<Mutex<usize>>,
    trigger: &'static str,
}
impl OrdinaryLifecycleHost for ReceiptFailureCodexHost {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn wake(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn drive(&mut self) -> Result<(), crate::ordinary_lifecycle_host::DriveFailure> {
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        false
    }
    fn respond_codex_permission(
        &self,
        _: &str,
        _: &str,
        _: gent_drivers::codex_control::CodexControlDecision,
        _: Option<serde_json::Value>,
    ) -> Result<(), Undelivered> {
        *self.writes.lock().unwrap() += 1;
        let db = rusqlite::Connection::open(&self.database).map_err(|_| Undelivered::PipeBroken)?;
        db.execute_batch(self.trigger)
            .map_err(|_| Undelivered::PipeBroken)?;
        Ok(())
    }
}

type ClaudeResponses = Arc<Mutex<Vec<(String, String, bool, Option<serde_json::Value>)>>>;

struct ClaudeHost(ClaudeResponses);

#[test]
fn decision_event_identity_is_scoped_to_the_run_and_turn() {
    let binding = |run: &str, turn: &str| PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("0".into()),
        request_idempotency_key: format!("codex:{run}:{turn}:0"),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId(run.into()),
        turn_id: turn.into(),
        policy_id: "policy".into(),
        policy_revision: 1,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    let first = permission_decision_accepted_event_id(&binding("run-a", "turn-1"));
    assert_ne!(
        first,
        permission_decision_accepted_event_id(&binding("run-b", "turn-1"))
    );
    assert_ne!(
        first,
        permission_decision_accepted_event_id(&binding("run-a", "turn-2"))
    );
}

impl OrdinaryLifecycleHost for ClaudeHost {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claude
    }
    fn arm_authority_recovery(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn wake(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn drive(&mut self) -> Result<(), crate::ordinary_lifecycle_host::DriveFailure> {
        Ok(())
    }
    fn needs_drive(&self) -> bool {
        false
    }
    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), Undelivered> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), Undelivered> {
        self.0.lock().unwrap().push((
            run_id.into(),
            request_id.into(),
            behavior == gent_drivers::claude_control::ClaudePermissionBehavior::Allow
                && persist_suggestions,
            updated_input,
        ));
        Ok(())
    }
}

const RECEIPT_TRIGGER: &str = "CREATE TRIGGER fail_receipt_update BEFORE UPDATE ON receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;";
const PENDING_TRIGGER: &str = "CREATE TRIGGER fail_pending_delete BEFORE DELETE ON pending_provider_permissions BEGIN SELECT RAISE(ABORT, 'injected pending failure'); END;";

fn codex_decision(ledger: &SqliteLedger) -> PermissionDecisionResponse {
    codex_decision_for(ledger, "Command")
}

fn codex_decision_for(ledger: &SqliteLedger, tool_name: &str) -> PermissionDecisionResponse {
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let policy = ledger
        .ensure_default_provider_permission_policy("workspace")
        .unwrap();
    let binding = PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("request".into()),
        request_idempotency_key: "codex:request".into(),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        turn_id: "turn".into(),
        policy_id: policy.policy_id,
        policy_revision: policy.revision,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    ledger
        .save_pending_permission(&pending(&binding, tool_name, None))
        .unwrap();
    PermissionDecisionResponse {
        binding,
        response: PermissionDecisionResponseKind::ApproveOnce,
        input: None,
        command_pattern: None,
    }
}

fn port_with(
    ledger: &SqliteLedger,
    host: Box<dyn OrdinaryLifecycleHost>,
) -> StandaloneAgentChatPermissionPort {
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), vec![host])
            .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress)
}

type Failing = (
    Box<dyn OrdinaryLifecycleHost>,
    Arc<Mutex<usize>>,
    Arc<Mutex<Vec<String>>>,
);

fn failing(failure: Undelivered) -> Failing {
    let writes = Arc::new(Mutex::new(0usize));
    let interrupts = Arc::new(Mutex::new(Vec::new()));
    let host = FailingCodexHost {
        writes: Arc::clone(&writes),
        failure,
        interrupts: Arc::clone(&interrupts),
    };
    (Box::new(host), writes, interrupts)
}

fn respond(response: &PermissionDecisionResponse) -> AgentChatPermissionFrame {
    AgentChatPermissionFrame::Respond {
        request_id: gent_types::AgentChatRequestId("ipc".into()),
        receipt_id: ReceiptId("receipt".into()),
        response: response.clone(),
    }
}

fn status_of(frame: &AgentChatPermissionFrame) -> gent_types::ReceiptStatus {
    match frame {
        AgentChatPermissionFrame::Accepted { receipt, .. } => receipt.status.clone(),
        other => panic!("unexpected permission frame {other:?}"),
    }
}

fn durable_status(database: &std::path::Path) -> String {
    rusqlite::Connection::open(database)
        .unwrap()
        .query_row(
            "SELECT status FROM receipts WHERE idempotency_key = 'codex:request'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn file_ledger() -> (tempfile::TempDir, std::path::PathBuf, SqliteLedger) {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("permission.sqlite");
    let ledger = SqliteLedger::open(&database).unwrap();
    (temp, database, ledger)
}

fn assert_cleared(ledger: &SqliteLedger, response: &PermissionDecisionResponse) {
    assert!(
        ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        ledger
            .read_conversation_activity_page("conversation", "run", 0, 64)
            .unwrap()
            .facts
            .as_slice(),
        [
            gent_types::ConversationActivityFact::DecisionPending { .. },
            gent_types::ConversationActivityFact::DecisionSettled { decision_id, .. }
        ] if decision_id == "request"
    ));
}

fn claim_first_attempt(ledger: &SqliteLedger, response: &PermissionDecisionResponse) {
    use gent_ports::Ledger;
    let (command, event) =
        super::receipt::permission_decision_command(response, &ReceiptId("receipt".into()))
            .unwrap();
    ledger.claim_command(&command, &event).unwrap();
}

#[tokio::test]
async fn retried_accepted_decision_without_its_pending_row_settles_without_a_write() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    ledger.settle_pending_permission(&response.binding).unwrap();
    let (host, writes, _) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "settled");
    assert_eq!(*writes.lock().unwrap(), 0);
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn retried_accepted_decision_still_pending_in_this_epoch_is_written_now() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    let relays = Arc::new(Mutex::new(Vec::new()));
    let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "settled");
    assert_eq!(relays.lock().unwrap().len(), 1);
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn decision_from_an_older_host_epoch_expires_without_a_write() {
    use super::AgentChatPermissionPort;
    use gent_ports::Ledger;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    let (host, writes, _) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Rejected);
    assert_eq!(durable_status(&database), "rejected");
    assert_eq!(*writes.lock().unwrap(), 0);
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn undeliverable_decision_expires_with_a_typed_error_code() {
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let (host, writes, interrupts) = failing(Undelivered::NotPending);
    let port = port_with(&ledger, host);
    let (mut reader, mut sink) = duplex(4096);
    for _ in 0..2 {
        let frame = serde_json::to_value(respond(&response)).unwrap();
        assert!(dispatch_port(&mut sink, &port, &frame).await.unwrap());
        let reply: gent_protocol::WireFrame = gent_protocol::read_frame(&mut reader).await.unwrap();
        assert!(
            matches!(&reply, gent_protocol::WireFrame::Error { code, .. } if code == "agentChatPermissionExpired"),
            "{reply:?}"
        );
    }
    assert_eq!(*writes.lock().unwrap(), 1);
    assert!(interrupts.lock().unwrap().is_empty());
    assert_eq!(durable_status(&database), "rejected");
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn write_that_fails_midway_terminates_the_run_and_rejects_the_decision() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let (host, writes, interrupts) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    for _ in 0..2 {
        let result = port.exchange(respond(&response)).await.unwrap();
        assert_eq!(status_of(&result), gent_types::ReceiptStatus::Rejected);
    }
    assert_eq!(*writes.lock().unwrap(), 1);
    assert_eq!(*interrupts.lock().unwrap(), ["run"]);
    assert_eq!(durable_status(&database), "rejected");
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn delivered_decision_reports_settled_when_its_receipt_cannot_be_stored() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let writes = Arc::new(Mutex::new(0usize));
    let port = port_with(
        &ledger,
        Box::new(ReceiptFailureCodexHost {
            database: database.clone(),
            writes: Arc::clone(&writes),
            trigger: RECEIPT_TRIGGER,
        }),
    );
    let first = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&first), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "accepted");
    assert_cleared(&ledger, &response);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_receipt_update")
        .unwrap();
    let recovered = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&recovered), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "settled");
    assert_eq!(*writes.lock().unwrap(), 1);
}

#[tokio::test]
async fn delivered_decision_is_settled_once_and_its_uncleared_row_ends_with_the_run() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let writes = Arc::new(Mutex::new(0usize));
    let port = port_with(
        &ledger,
        Box::new(ReceiptFailureCodexHost {
            database: database.clone(),
            writes: Arc::clone(&writes),
            trigger: PENDING_TRIGGER,
        }),
    );
    let first = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&first), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "settled");
    assert!(
        ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .unwrap()
            .is_some()
    );
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_pending_delete")
        .unwrap();
    let retried = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&retried), gent_types::ReceiptStatus::Settled);
    assert_eq!(*writes.lock().unwrap(), 1);
    end_run(&ledger);
    assert_cleared(&ledger, &response);
}

fn end_run(ledger: &SqliteLedger) {
    use gent_ports::Ledger;
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: "run".into(),
            coordinator_id: "daemon".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    crate::provider_effects::ProviderEffectDispatcher::new(
        gent_runtime::Coordinator::new(ledger.clone(), gent_types::CapabilitySet::default()),
        gent_runtime::ProviderRunAuthority::PublicDrivers,
    )
    .record(
        "exit".into(),
        "run",
        "daemon",
        HostEpoch(1),
        &gent_drivers::SessionEffect::Terminal {
            reason: "providerExited:1".into(),
        },
    )
    .unwrap();
}

#[tokio::test]
async fn invalid_answer_input_is_a_typed_rejection_and_the_request_stays_pending() {
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let (host, writes, interrupts) = failing(Undelivered::InvalidInput);
    let port = port_with(&ledger, host);
    let (mut reader, mut sink) = duplex(4096);
    let frame = serde_json::to_value(respond(&response)).unwrap();
    assert!(dispatch_port(&mut sink, &port, &frame).await.unwrap());
    let reply: gent_protocol::WireFrame = gent_protocol::read_frame(&mut reader).await.unwrap();
    assert!(
        matches!(&reply, gent_protocol::WireFrame::Error { code, .. } if code == "agentChatPermissionInputInvalid"),
        "{reply:?}"
    );
    assert_eq!(*writes.lock().unwrap(), 1);
    assert!(interrupts.lock().unwrap().is_empty());
    assert_eq!(durable_status(&database), "rejected");
    assert_eq!(
        ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .unwrap(),
        Some(pending(&response.binding, "Command", None))
    );
}

#[tokio::test]
async fn an_always_allow_grant_is_written_only_when_its_answer_is_delivered() {
    use super::AgentChatPermissionPort;
    let policy = |ledger: &SqliteLedger| {
        ledger
            .current_policy("workspace", gent_types::PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap()
    };
    for failure in [
        Undelivered::NotPending,
        Undelivered::InvalidInput,
        Undelivered::PipeBroken,
    ] {
        let (_temp, _database, ledger) = file_ledger();
        let mut response = codex_decision(&ledger);
        response.response = PermissionDecisionResponseKind::ApproveExactTool;
        let before = policy(&ledger);
        let (host, writes, _) = failing(failure);
        let _ = port_with(&ledger, host).exchange(respond(&response)).await;
        assert_eq!(*writes.lock().unwrap(), 1);
        assert_eq!(policy(&ledger), before, "{failure:?}");
    }
    let (_temp, _database, ledger) = file_ledger();
    let mut response = codex_decision(&ledger);
    response.response = PermissionDecisionResponseKind::ApproveExactTool;
    let relays = Arc::new(Mutex::new(Vec::new()));
    let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Settled);
    assert_eq!(policy(&ledger).allowed_tools, ["Command"]);
    assert_cleared(&ledger, &response);
}

#[test]
fn provider_run_terminal_clears_a_pending_permission_left_after_its_turn() {
    use gent_ports::Ledger;
    let ledger = SqliteLedger::in_memory().unwrap();
    let response = codex_decision(&ledger);
    ledger
        .claim_run_lease(&gent_ports::RunLease {
            run_id: "run".into(),
            coordinator_id: "daemon".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    let dispatcher = crate::provider_effects::ProviderEffectDispatcher::new(
        gent_runtime::Coordinator::new(ledger.clone(), gent_types::CapabilitySet::default()),
        gent_runtime::ProviderRunAuthority::PublicDrivers,
    );
    dispatcher
        .record(
            "exit".into(),
            "run",
            "daemon",
            HostEpoch(1),
            &gent_drivers::SessionEffect::Terminal {
                reason: "providerExited:1".into(),
            },
        )
        .unwrap();
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn codex_response_is_receipted_and_relayed_through_typed_ipc() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let policy = ledger
        .ensure_default_provider_permission_policy("workspace")
        .unwrap();
    let binding = PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("request".into()),
        request_idempotency_key: "codex:request".into(),
        conversation_id: conversation,
        run_id: AgentChatRunId("run".into()),
        turn_id: "turn".into(),
        policy_id: policy.policy_id,
        policy_revision: policy.revision,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    ledger
        .save_pending_permission(&pending(
            &binding,
            "Command",
            Some(serde_json::json!({"kind":"questions","questions":[{"id":"q1"}]})),
        ))
        .unwrap();
    assert!(matches!(
        ledger
            .read_conversation_activity_page("conversation", "run", 0, 64)
            .unwrap()
            .facts
            .as_slice(),
        [gent_types::ConversationActivityFact::DecisionPending { decision_id, .. }]
            if decision_id == "request"
    ));
    let relays = Arc::new(Mutex::new(Vec::new()));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(CodexHost(Arc::clone(&relays)))],
        )
        .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress);
    let pending = AgentChatPermissionFrame::PendingRead {
        request_id: gent_types::AgentChatRequestId("pending".into()),
        conversation_id: binding.conversation_id.clone(),
    };
    let frame = AgentChatPermissionFrame::Respond {
        request_id: gent_types::AgentChatRequestId("ipc".into()),
        receipt_id: ReceiptId("receipt".into()),
        response: PermissionDecisionResponse {
            binding: binding.clone(),
            response: PermissionDecisionResponseKind::ApproveOnce,
            input: Some(serde_json::json!({"q1":"A"})),
            command_pattern: None,
        },
    };
    let (mut reader, mut writer) = duplex(4096);
    dispatch_port(&mut writer, &port, &serde_json::to_value(pending).unwrap())
        .await
        .unwrap();
    let pending_response = read_json_frame::<_, AgentChatPermissionFrame>(&mut reader)
        .await
        .unwrap();
    assert!(
        matches!(pending_response, AgentChatPermissionFrame::Pending { request: Some(request), .. } if request.request.input == Some(serde_json::json!({"kind":"questions","questions":[{"id":"q1"}]})))
    );
    assert!(
        dispatch_port(&mut writer, &port, &serde_json::to_value(frame).unwrap())
            .await
            .unwrap()
    );
    assert!(matches!(
        read_json_frame::<_, AgentChatPermissionFrame>(&mut reader)
            .await
            .unwrap(),
        AgentChatPermissionFrame::Accepted { .. }
    ));
    assert_eq!(
        *relays.lock().unwrap(),
        vec![(
            "run".into(),
            "request".into(),
            Some(serde_json::json!({"q1":"A"}))
        )]
    );
    assert!(
        ledger
            .pending_permission(&binding.conversation_id, &binding.run_id)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        ledger
            .read_conversation_activity_page("conversation", "run", 0, 64)
            .unwrap()
            .facts
            .as_slice(),
        [
            gent_types::ConversationActivityFact::DecisionPending { .. },
            gent_types::ConversationActivityFact::DecisionSettled { decision_id, .. }
        ] if decision_id == "request"
    ));
}

#[tokio::test]
async fn claude_response_is_receipted_without_forwarding_provider_grants() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-sonnet".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let policy = ledger
        .ensure_default_provider_permission_policy("workspace")
        .unwrap();
    let binding = PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("request".into()),
        request_idempotency_key: "claude:request".into(),
        conversation_id: conversation,
        run_id: AgentChatRunId("run".into()),
        turn_id: "turn".into(),
        policy_id: policy.policy_id,
        policy_revision: policy.revision,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    ledger
        .save_pending_permission(&pending(&binding, "Bash", None))
        .unwrap();
    let relays = Arc::new(Mutex::new(Vec::new()));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(ClaudeHost(Arc::clone(&relays)))],
        )
        .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress);
    let response = PermissionDecisionResponse {
        binding: binding.clone(),
        response: PermissionDecisionResponseKind::ApproveExactTool,
        input: Some(serde_json::json!({"plan":"approved"})),
        command_pattern: None,
    };
    let receipt = port
        .respond_claude_with_receipt(&response, &ReceiptId("receipt".into()))
        .unwrap();
    assert_eq!(receipt.status, gent_types::ReceiptStatus::Settled);
    assert_eq!(
        *relays.lock().unwrap(),
        vec![(
            "run".into(),
            "request".into(),
            false,
            Some(serde_json::json!({"plan":"approved"}))
        )]
    );
    assert!(
        ledger
            .pending_permission(&binding.conversation_id, &binding.run_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn claude_scoped_approvals_do_not_persist_provider_suggestions() {
    for choice in [
        PermissionDecisionResponseKind::ApproveExactTool,
        PermissionDecisionResponseKind::ApproveCategory,
    ] {
        let (ledger, port, mut response, _, claude) =
            canonical_grant_fixture(AgentChatProvider::Claude);
        response.response = choice;
        let receipt = port
            .respond_claude_with_receipt(&response, &ReceiptId("scoped".into()))
            .unwrap();
        assert_eq!(receipt.status, gent_types::ReceiptStatus::Settled);
        assert_eq!(claude.lock().unwrap().len(), 1);
        assert!(!claude.lock().unwrap()[0].2);
        assert_eq!(
            ledger
                .current_policy("workspace", gent_types::PolicyScope::ProviderPermissions)
                .unwrap()
                .unwrap()
                .revision,
            2
        );
    }
}

fn canonical_grant_fixture(
    provider: AgentChatProvider,
) -> (
    SqliteLedger,
    StandaloneAgentChatPermissionPort,
    PermissionDecisionResponse,
    CodexResponses,
    ClaudeResponses,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider,
                    model: "fixture".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    ledger
        .save_policy(&gent_types::PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: gent_types::PolicyScope::ProviderPermissions,
            revision: 1,
            mode: gent_types::PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
            allowed_commands: Vec::new(),
        })
        .unwrap();
    let binding = PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("request".into()),
        request_idempotency_key: "permission-request".into(),
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("run".into()),
        turn_id: "turn".into(),
        policy_id: "policy".into(),
        policy_revision: 1,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    ledger
        .save_pending_permission(&pending(&binding, "ExternalWrite", None))
        .unwrap();
    let codex = Arc::new(Mutex::new(Vec::new()));
    let claude = Arc::new(Mutex::new(Vec::new()));
    let hosts: Vec<Box<dyn OrdinaryLifecycleHost>> = match provider {
        AgentChatProvider::Codex => vec![Box::new(CodexHost(codex.clone()))],
        AgentChatProvider::Claude => vec![Box::new(ClaudeHost(claude.clone()))],
        AgentChatProvider::Claurst => panic!("Claurst has its separate bridge fixture"),
    };
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), hosts)
            .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress);
    let response = PermissionDecisionResponse {
        binding,
        response: PermissionDecisionResponseKind::ApproveExactTool,
        input: None,
        command_pattern: None,
    };
    (ledger, port, response, codex, claude)
}

#[test]
fn codex_and_claude_commit_the_same_canonical_grant_and_answer_after_a_newer_revision() {
    for provider in [AgentChatProvider::Codex, AgentChatProvider::Claude] {
        let (ledger, port, response, _, _) = canonical_grant_fixture(provider);
        let receipt = if provider == AgentChatProvider::Codex {
            port.respond_codex_with_receipt(&response, &ReceiptId("approval".into()))
        } else {
            port.respond_claude_with_receipt(&response, &ReceiptId("approval".into()))
        }
        .unwrap();
        assert_eq!(receipt.status, gent_types::ReceiptStatus::Settled);
        let committed = ledger
            .current_policy("workspace", gent_types::PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap();
        assert_eq!(
            committed.policy_id,
            "provider-permissions-v2-542bc547a33628f5c866fdce8a982efe59831a79149e94ff8ce7c8ab95b0dbed"
        );
        assert_eq!(committed.allowed_tools, ["ExternalWrite"]);
        let (ledger, port, mut response, codex, claude) = canonical_grant_fixture(provider);
        let mut current = ledger
            .current_policy("workspace", gent_types::PolicyScope::ProviderPermissions)
            .unwrap()
            .unwrap();
        current.revision = 2;
        current.policy_id = "newer-policy".into();
        ledger.save_policy(&current).unwrap();
        response.response = PermissionDecisionResponseKind::ApproveOnce;
        let result = if provider == AgentChatProvider::Codex {
            port.respond_codex_with_receipt(&response, &ReceiptId("stale-approval".into()))
        } else {
            port.respond_claude_with_receipt(&response, &ReceiptId("stale-approval".into()))
        };
        assert_eq!(result.unwrap().status, gent_types::ReceiptStatus::Settled);
        assert_eq!(
            codex.lock().unwrap().len() + claude.lock().unwrap().len(),
            1
        );
        assert_eq!(
            ledger
                .current_policy("workspace", gent_types::PolicyScope::ProviderPermissions)
                .unwrap()
                .unwrap()
                .policy_id,
            "newer-policy"
        );
    }
}

#[tokio::test]
async fn pending_permission_of_a_running_turn_is_readable_after_a_mid_turn_switch() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation = AgentChatConversationId("conversation".into());
    let selection = |mode| AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen".into(),
        effort: AgentChatEffort::Medium,
        mode,
    };
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("conversation-receipt".into()),
                idempotency_key: "conversation-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: AgentChatRunId("run".into()),
                selection: selection(AgentChatMode::Ask),
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let policy = ledger
        .ensure_default_provider_permission_policy("workspace")
        .unwrap();
    let binding = PermissionDecisionBinding {
        decision_id: AgentChatDecisionId("claurst:n:1".into()),
        request_idempotency_key: "claurst:request".into(),
        conversation_id: conversation.clone(),
        run_id: AgentChatRunId("run".into()),
        turn_id: "turn".into(),
        policy_id: policy.policy_id,
        policy_revision: policy.revision,
        host_epoch: HostEpoch(1),
        request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
    };
    ledger
        .save_pending_permission(&pending(&binding, "Edit", None))
        .unwrap();
    ledger
        .switch_agent_chat_selection(&AgentChatSelectionSwitch {
            receipt_id: ReceiptId("switch".into()),
            idempotency_key: "switch".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation.clone(),
            parent_run_id: AgentChatRunId("run".into()),
            run_id: AgentChatRunId("switched".into()),
            selection: selection(AgentChatMode::Agent),
            context_policy: ContextPolicy::Preserve,
        })
        .unwrap();
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), Vec::new())
            .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress);
    assert_eq!(
        ledger
            .read_agent_chat_detail("conversation")
            .unwrap()
            .current_run_id,
        "switched"
    );
    let reply = super::AgentChatPermissionPort::exchange(
        &port,
        AgentChatPermissionFrame::PendingRead {
            request_id: gent_types::AgentChatRequestId("pending".into()),
            conversation_id: conversation,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        AgentChatPermissionFrame::Pending { request: Some(request), .. } if request.binding == binding
    ));
}

fn respond_as(response: &PermissionDecisionResponse, receipt: &str) -> AgentChatPermissionFrame {
    AgentChatPermissionFrame::Respond {
        request_id: gent_types::AgentChatRequestId("ipc".into()),
        receipt_id: ReceiptId(receipt.into()),
        response: response.clone(),
    }
}

fn denial(response: &PermissionDecisionResponse) -> PermissionDecisionResponse {
    PermissionDecisionResponse {
        response: PermissionDecisionResponseKind::Deny,
        ..response.clone()
    }
}

async fn wire_reply(
    port: &StandaloneAgentChatPermissionPort,
    frame: &AgentChatPermissionFrame,
) -> serde_json::Value {
    let (mut reader, mut sink) = duplex(4096);
    let raw = serde_json::to_value(frame).unwrap();
    assert!(dispatch_port(&mut sink, port, &raw).await.unwrap());
    read_json_frame::<_, serde_json::Value>(&mut reader)
        .await
        .unwrap()
}

fn is_expired(reply: &serde_json::Value) -> bool {
    matches!(
        serde_json::from_value::<gent_protocol::WireFrame>(reply.clone()),
        Ok(gent_protocol::WireFrame::Error { code, .. }) if code == "agentChatPermissionExpired"
    )
}

fn settle_first_attempt(ledger: &SqliteLedger, status: gent_types::ReceiptStatus) {
    use gent_ports::Ledger;
    let event = gent_types::Event {
        cursor: 0,
        event_id: "first-terminal".into(),
        receipt_id: ReceiptId("receipt".into()),
        host_epoch: HostEpoch(1),
        kind: "agentChatPermissionDecisionTerminal".into(),
        payload: serde_json::json!({}),
    };
    ledger
        .settle_receipt("codex:request", status, &event)
        .unwrap();
}

#[tokio::test]
async fn a_different_answer_after_delivery_reports_the_delivered_outcome_without_a_write() {
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    let relays = Arc::new(Mutex::new(Vec::new()));
    let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
    let first = {
        use super::AgentChatPermissionPort;
        port.exchange(respond(&response)).await.unwrap()
    };
    assert_eq!(status_of(&first), gent_types::ReceiptStatus::Settled);
    let reply = wire_reply(&port, &respond_as(&denial(&response), "second")).await;
    let Ok(AgentChatPermissionFrame::Accepted { receipt, .. }) =
        serde_json::from_value::<AgentChatPermissionFrame>(reply.clone())
    else {
        panic!("unexpected reply {reply}");
    };
    assert_eq!(receipt.status, gent_types::ReceiptStatus::Settled);
    assert_eq!(receipt.receipt_id.0, "receipt");
    assert_eq!(relays.lock().unwrap().len(), 1);
    assert_eq!(durable_status(&database), "settled");
}

#[tokio::test]
async fn a_different_answer_after_the_request_expired_gets_the_typed_expired_code() {
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    settle_first_attempt(&ledger, gent_types::ReceiptStatus::Rejected);
    ledger.settle_pending_permission(&response.binding).unwrap();
    let (host, writes, _) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    let reply = wire_reply(&port, &respond_as(&denial(&response), "second")).await;
    assert!(is_expired(&reply), "{reply}");
    assert_eq!(*writes.lock().unwrap(), 0);
    assert_eq!(durable_status(&database), "rejected");
}

#[tokio::test]
async fn a_different_answer_while_the_first_was_never_written_is_delivered() {
    use super::AgentChatPermissionPort;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    settle_first_attempt(&ledger, gent_types::ReceiptStatus::Rejected);
    let relays = Arc::new(Mutex::new(Vec::new()));
    let port = port_with(&ledger, Box::new(CodexHost(Arc::clone(&relays))));
    let result = port
        .exchange(respond_as(&denial(&response), "second"))
        .await
        .unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Settled);
    assert_eq!(relays.lock().unwrap().len(), 1);
    assert_eq!(durable_status(&database), "rejected");
    assert_cleared(&ledger, &response);
}

#[tokio::test]
async fn a_different_answer_while_the_first_still_owns_the_request_is_not_written() {
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    let (host, writes, _) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    let reply = wire_reply(&port, &respond_as(&denial(&response), "second")).await;
    assert!(is_expired(&reply), "{reply}");
    assert_eq!(*writes.lock().unwrap(), 0);
    assert_eq!(durable_status(&database), "accepted");
    assert!(
        ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_delivered_decision_resent_after_a_host_restart_still_reports_settled() {
    use super::AgentChatPermissionPort;
    use gent_ports::Ledger;
    let (_temp, database, ledger) = file_ledger();
    let response = codex_decision(&ledger);
    claim_first_attempt(&ledger, &response);
    settle_first_attempt(&ledger, gent_types::ReceiptStatus::Settled);
    ledger.settle_pending_permission(&response.binding).unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    let (host, writes, _) = failing(Undelivered::PipeBroken);
    let port = port_with(&ledger, host);
    let result = port.exchange(respond(&response)).await.unwrap();
    assert_eq!(status_of(&result), gent_types::ReceiptStatus::Settled);
    assert_eq!(durable_status(&database), "settled");
    assert_eq!(*writes.lock().unwrap(), 0);
}

#[path = "agent_chat_permission_command_grant_tests.rs"]
mod command_grants;

#[tokio::test]
async fn a_question_approved_without_its_answers_is_refused_and_stays_pending() {
    let (_temp, database, ledger) = file_ledger();
    let mut response = codex_decision_for(&ledger, "AskUserQuestion");
    let question = pending(&response.binding, "AskUserQuestion", None);
    let writes = CodexResponses::default();
    let port = port_with(&ledger, Box::new(CodexHost(writes.clone())));
    let (mut reader, mut sink) = duplex(4096);
    let frame = serde_json::to_value(respond(&response)).unwrap();
    assert!(dispatch_port(&mut sink, &port, &frame).await.unwrap());
    let reply: gent_protocol::WireFrame = gent_protocol::read_frame(&mut reader).await.unwrap();
    assert!(
        matches!(&reply, gent_protocol::WireFrame::Error { code, .. } if code == "agentChatPermissionInputInvalid"),
        "{reply:?}"
    );
    assert!(writes.lock().unwrap().is_empty());
    assert_eq!(durable_status(&database), "rejected");
    assert_eq!(
        ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .unwrap(),
        Some(question)
    );
    response.input = Some(serde_json::json!({"answers": {"Which?": "A"}}));
    let frame = serde_json::to_value(respond_as(&response, "answered")).unwrap();
    assert!(dispatch_port(&mut sink, &port, &frame).await.unwrap());
    let answered: AgentChatPermissionFrame = read_json_frame(&mut reader).await.unwrap();
    assert_eq!(status_of(&answered), gent_types::ReceiptStatus::Settled);
    assert_eq!(
        writes.lock().unwrap().as_slice(),
        [(
            "run".to_owned(),
            "request".to_owned(),
            Some(serde_json::json!({"answers": {"Which?": "A"}}))
        )]
    );
}
