use std::sync::{Arc, Mutex};

use gent_ports::{
    AgentChatWorkspaceLedger, ConversationActivityLedger, PendingPermissionLedger, PolicyLedger,
};
use gent_protocol::{AgentChatPermissionFrame, read_json_frame};
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatDecisionId, AgentChatEffort,
    AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection, HostEpoch,
    PermissionCategory, PermissionDecisionBinding, PermissionDecisionRequest,
    PermissionDecisionResponse, PermissionDecisionResponseKind, PermissionRequest,
    PermissionRequestDigest, ReceiptId, WorkspaceRecord,
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

struct CodexHost(Arc<Mutex<Vec<(String, String, Option<serde_json::Value>)>>>);

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

struct ClaudeHost(Arc<Mutex<Vec<(String, String, bool, Option<serde_json::Value>)>>>);

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
        run_id: binding.run_id.clone(),
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
async fn claude_response_is_receipted_and_relays_persistent_intent() {
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
            true,
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
