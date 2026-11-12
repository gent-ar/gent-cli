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
    fn drive(&mut self) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
        self.0
            .lock()
            .unwrap()
            .push((run_id.into(), request_id.into(), answers));
        Ok(())
    }
}

struct FailingCodexHost(Arc<Mutex<usize>>);
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
    fn drive(&mut self) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
        *self.0.lock().unwrap() += 1;
        Err(())
    }
}

struct ReceiptFailureCodexHost {
    database: std::path::PathBuf,
    writes: Arc<Mutex<usize>>,
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
    fn drive(&mut self) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
        *self.writes.lock().unwrap() += 1;
        let db = rusqlite::Connection::open(&self.database).map_err(|_| ())?;
        db.execute_batch("CREATE TRIGGER fail_receipt_update BEFORE UPDATE ON receipts BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;").map_err(|_| ())?;
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
    fn drive(&mut self) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
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
    ) -> Result<(), ()> {
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

#[tokio::test]
async fn accepted_receipt_with_missing_pending_permission_recovers_as_unprovable() {
    use super::AgentChatPermissionPort;
    use gent_ports::Ledger;
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
    let response = PermissionDecisionResponse {
        binding: binding.clone(),
        response: PermissionDecisionResponseKind::ApproveOnce,
        input: None,
    };
    let receipt_id = ReceiptId("recovery-receipt".into());
    let (command, event) =
        super::receipt::permission_decision_command(&response, &receipt_id).unwrap();
    ledger.claim_command(&command, &event).unwrap();
    ledger.settle_pending_permission(&binding).unwrap();
    let writes = Arc::new(Mutex::new(0usize));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(FailingCodexHost(Arc::clone(&writes)))],
        )
        .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger, ingress);
    let result = port
        .exchange(AgentChatPermissionFrame::Respond {
            request_id: gent_types::AgentChatRequestId("recovery".into()),
            receipt_id,
            response,
        })
        .await
        .unwrap();
    assert!(
        matches!(result, AgentChatPermissionFrame::Accepted { receipt, .. }
        if receipt.status == gent_types::ReceiptStatus::Unprovable)
    );
    assert_eq!(*writes.lock().unwrap(), 0);
}

#[tokio::test]
async fn receipt_storage_failure_after_provider_write_reports_unknown_and_never_replays() {
    use super::AgentChatPermissionPort;
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("receipt-failure.sqlite");
    let ledger = SqliteLedger::open(&database).unwrap();
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
    let writes = Arc::new(Mutex::new(0usize));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(ReceiptFailureCodexHost {
                database: database.clone(),
                writes: Arc::clone(&writes),
            })],
        )
        .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger, ingress);
    let frame = AgentChatPermissionFrame::Respond {
        request_id: gent_types::AgentChatRequestId("ipc".into()),
        receipt_id: ReceiptId("receipt".into()),
        response: PermissionDecisionResponse {
            binding,
            response: PermissionDecisionResponseKind::ApproveOnce,
            input: None,
        },
    };
    let first = port.exchange(frame.clone()).await.unwrap();
    assert!(
        matches!(first, AgentChatPermissionFrame::Accepted { receipt, .. } if receipt.status == gent_types::ReceiptStatus::Unprovable)
    );
    assert_eq!(*writes.lock().unwrap(), 1);
    let db = rusqlite::Connection::open(&database).unwrap();
    let status: String = db
        .query_row(
            "SELECT status FROM receipts WHERE idempotency_key = 'codex:request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "accepted",
        "the fallback must not pretend it was stored"
    );
    db.execute_batch("DROP TRIGGER fail_receipt_update")
        .unwrap();
    let recovered = port.exchange(frame).await.unwrap();
    assert!(
        matches!(recovered, AgentChatPermissionFrame::Accepted { receipt, .. } if receipt.status == gent_types::ReceiptStatus::Unprovable)
    );
    assert_eq!(*writes.lock().unwrap(), 1);
}

#[tokio::test]
async fn codex_unconfirmed_provider_write_returns_unprovable_receipt_without_replay() {
    use super::AgentChatPermissionPort;
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
    let writes = Arc::new(Mutex::new(0usize));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(
            AgentChatReadService::new(ledger.clone()),
            vec![Box::new(FailingCodexHost(Arc::clone(&writes)))],
        )
        .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_readiness(router, ledger.clone(), HostEpoch(1));
    let port = StandaloneAgentChatPermissionPort::new(ledger.clone(), ingress);
    let frame = AgentChatPermissionFrame::Respond {
        request_id: gent_types::AgentChatRequestId("ipc".into()),
        receipt_id: ReceiptId("receipt".into()),
        response: PermissionDecisionResponse {
            binding,
            response: PermissionDecisionResponseKind::ApproveOnce,
            input: None,
        },
    };
    for _ in 0..2 {
        let result = port.exchange(frame.clone()).await.unwrap();
        assert!(
            matches!(result, AgentChatPermissionFrame::Accepted { receipt, .. }
            if receipt.status == gent_types::ReceiptStatus::Unprovable)
        );
    }
    assert_eq!(*writes.lock().unwrap(), 1);
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
