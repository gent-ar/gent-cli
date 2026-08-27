use async_trait::async_trait;
use gent_ports::{
    AgentChatReadLedger, AgentChatWorkspaceLedger, Ledger, PendingPermissionLedger, PolicyLedger,
    ReceiptClaim,
};
use gent_protocol::AgentChatPermissionFrame;
use gent_store::SqliteLedger;
use gent_types::{
    Command, Event, PermissionDecisionResponse, PermissionDecisionResponseKind, PolicyScope,
    Receipt, ReceiptId, ReceiptStatus,
};
use sha2::{Digest, Sha256};

use crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress;

#[async_trait]
pub(crate) trait AgentChatPermissionPort: Send + Sync {
    async fn exchange(
        &self,
        frame: AgentChatPermissionFrame,
    ) -> Result<AgentChatPermissionFrame, String>;
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneAgentChatPermissionPort {
    ledger: SqliteLedger,
    ingress: OrdinaryPromptIngress<SqliteLedger>,
}

impl StandaloneAgentChatPermissionPort {
    #[must_use]
    pub(crate) fn new(ledger: SqliteLedger, ingress: OrdinaryPromptIngress<SqliteLedger>) -> Self {
        Self { ledger, ingress }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gent_ports::{AgentChatWorkspaceLedger, PendingPermissionLedger, PolicyLedger};
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

    use super::StandaloneAgentChatPermissionPort;
    use crate::{
        agent_chat_permission_transport::dispatch_port,
        ordinary_lifecycle_cadence::pair_with_standalone_readiness,
        ordinary_lifecycle_router::{OrdinaryLifecycleHost, OrdinaryPublicLifecycleRouter},
    };

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
            .save_pending_permission(&PermissionDecisionRequest {
                binding: binding.clone(),
                request: PermissionRequest {
                    tool_name: "Command".into(),
                    category: PermissionCategory::Command,
                    input: Some(serde_json::json!({"kind":"questions","questions":[{"id":"q1"}]})),
                },
            })
            .unwrap();
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
            .save_pending_permission(&PermissionDecisionRequest {
                binding: binding.clone(),
                request: PermissionRequest {
                    tool_name: "Bash".into(),
                    category: PermissionCategory::Command,
                    input: None,
                },
            })
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
}

#[async_trait]
impl AgentChatPermissionPort for StandaloneAgentChatPermissionPort {
    async fn exchange(
        &self,
        frame: AgentChatPermissionFrame,
    ) -> Result<AgentChatPermissionFrame, String> {
        match frame {
            AgentChatPermissionFrame::PendingRead {
                request_id,
                conversation_id,
                run_id,
            } => {
                let request = self
                    .ledger
                    .pending_permission(&conversation_id, &run_id)
                    .map_err(|error| error.to_string())?;
                Ok(AgentChatPermissionFrame::Pending {
                    request_id,
                    request,
                })
            }
            AgentChatPermissionFrame::Respond {
                request_id,
                receipt_id,
                response,
            } => {
                validate_response_input(response.input.as_ref())?;
                if response.binding.conversation_id.0.is_empty()
                    || response.binding.run_id.0.is_empty()
                {
                    return Err("permission response binding is invalid".into());
                }
                let receipt = match self.provider_for(&response)? {
                    gent_types::AgentChatProvider::Claurst => {
                        self.ingress
                            .respond_claurst_permission_with_receipt(response.clone(), receipt_id)
                            .await?
                    }
                    gent_types::AgentChatProvider::Codex => {
                        self.respond_codex_with_receipt(&response, &receipt_id)?
                    }
                    gent_types::AgentChatProvider::Claude => {
                        self.respond_claude_with_receipt(&response, &receipt_id)?
                    }
                };
                Ok(AgentChatPermissionFrame::Accepted {
                    request_id,
                    receipt,
                    decision_id: response.binding.decision_id.0,
                })
            }
            AgentChatPermissionFrame::Pending { .. }
            | AgentChatPermissionFrame::Accepted { .. } => {
                Err("permission response frames are server-only".into())
            }
        }
    }
}

fn validate_response_input(input: Option<&serde_json::Value>) -> Result<(), String> {
    let Some(input) = input else {
        return Ok(());
    };
    if !input.is_object() {
        return Err("permission response input must be an object".into());
    }
    let bytes = serde_json::to_vec(input).map_err(|error| error.to_string())?;
    if bytes.len() > 16 * 1024 {
        return Err("permission response input exceeds the bounded limit".into());
    }
    Ok(())
}

impl StandaloneAgentChatPermissionPort {
    fn provider_for(
        &self,
        response: &PermissionDecisionResponse,
    ) -> Result<gent_types::AgentChatProvider, String> {
        let detail = self
            .ledger
            .read_agent_chat_detail(&response.binding.conversation_id.0)
            .map_err(|error| error.to_string())?;
        detail
            .runs
            .into_iter()
            .find(|run| run.run_id == response.binding.run_id.0)
            .map(|run| run.selection.provider)
            .ok_or_else(|| "permission response run is unavailable".into())
    }

    fn respond_codex_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, String> {
        let command = Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: response.binding.request_idempotency_key.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecision".into(),
            payload: serde_json::to_value(response).map_err(|error| error.to_string())?,
        };
        let accepted = Event {
            cursor: 0,
            event_id: format!(
                "permission-decision-accepted:{}",
                response.binding.decision_id.0
            ),
            receipt_id: receipt_id.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecisionAccepted".into(),
            payload: command.payload.clone(),
        };
        match self
            .ledger
            .claim_command(&command, &accepted)
            .map_err(|error| error.to_string())?
        {
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                let pending = self
                    .ledger
                    .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
                    .map_err(|error| error.to_string())?;
                if pending
                    .as_ref()
                    .is_some_and(|request| request.binding == response.binding)
                {
                    return self.settle_receipt(&command, ReceiptStatus::Unprovable);
                }
                Err("Codex permission receipt recovery is stale".into())
            }
            ReceiptClaim::Existing(receipt) => Ok(receipt),
            ReceiptClaim::Accepted(_) => {
                if let Err(error) = self.respond_codex(response) {
                    let _ = self.settle_receipt(&command, ReceiptStatus::Unprovable);
                    return Err(error);
                }
                self.settle_receipt(&command, ReceiptStatus::Settled)
            }
        }
    }

    fn respond_claude_with_receipt(
        &self,
        response: &PermissionDecisionResponse,
        receipt_id: &ReceiptId,
    ) -> Result<Receipt, String> {
        let command = Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: response.binding.request_idempotency_key.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecision".into(),
            payload: serde_json::to_value(response).map_err(|error| error.to_string())?,
        };
        let accepted = Event {
            cursor: 0,
            event_id: format!(
                "permission-decision-accepted:{}",
                response.binding.decision_id.0
            ),
            receipt_id: receipt_id.clone(),
            host_epoch: response.binding.host_epoch,
            kind: "agentChatPermissionDecisionAccepted".into(),
            payload: command.payload.clone(),
        };
        match self
            .ledger
            .claim_command(&command, &accepted)
            .map_err(|error| error.to_string())?
        {
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                let pending = self
                    .ledger
                    .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
                    .map_err(|error| error.to_string())?;
                if pending
                    .as_ref()
                    .is_some_and(|request| request.binding == response.binding)
                {
                    return self.settle_receipt(&command, ReceiptStatus::Unprovable);
                }
                Err("Claude permission receipt recovery is stale".into())
            }
            ReceiptClaim::Existing(receipt) => Ok(receipt),
            ReceiptClaim::Accepted(_) => {
                if let Err(error) = self.respond_claude(response) {
                    let _ = self.settle_receipt(&command, ReceiptStatus::Unprovable);
                    return Err(error);
                }
                self.settle_receipt(&command, ReceiptStatus::Settled)
            }
        }
    }

    fn respond_codex(&self, response: &PermissionDecisionResponse) -> Result<(), String> {
        let pending = self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Codex permission is not pending".to_owned())?;
        if pending.binding != response.binding {
            return Err("Codex permission binding is stale".into());
        }
        self.persist_approval(&pending, response.response)?;
        let decision = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::codex_control::CodexControlDecision::Deny
        } else {
            gent_drivers::codex_control::CodexControlDecision::Allow
        };
        self.ingress.respond_codex_permission(
            &response.binding.run_id.0,
            &response.binding.decision_id.0,
            decision,
            response.input.clone(),
        )?;
        self.ledger
            .settle_pending_permission(&response.binding)
            .map_err(|error| error.to_string())
    }

    fn respond_claude(&self, response: &PermissionDecisionResponse) -> Result<(), String> {
        let pending = self
            .ledger
            .pending_permission(&response.binding.conversation_id, &response.binding.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Claude permission is not pending".to_owned())?;
        if pending.binding != response.binding {
            return Err("Claude permission binding is stale".into());
        }
        self.persist_approval(&pending, response.response)?;
        let behavior = if response.response == PermissionDecisionResponseKind::Deny {
            gent_drivers::claude_control::ClaudePermissionBehavior::Deny
        } else {
            gent_drivers::claude_control::ClaudePermissionBehavior::Allow
        };
        let persist_suggestions = matches!(
            response.response,
            PermissionDecisionResponseKind::ApproveExactTool
                | PermissionDecisionResponseKind::ApproveCategory
        );
        self.ingress.respond_claude_permission_with_input(
            &response.binding.run_id.0,
            &response.binding.decision_id.0,
            behavior,
            persist_suggestions,
            response.input.clone(),
        )?;
        self.ledger
            .settle_pending_permission(&response.binding)
            .map_err(|error| error.to_string())
    }

    fn persist_approval(
        &self,
        pending: &gent_types::PermissionDecisionRequest,
        response: PermissionDecisionResponseKind,
    ) -> Result<(), String> {
        if matches!(
            response,
            PermissionDecisionResponseKind::Deny | PermissionDecisionResponseKind::ApproveOnce
        ) {
            return Ok(());
        }
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(
                &pending.binding.conversation_id.0,
                &pending.binding.run_id.0,
            )
            .map_err(|error| error.to_string())?;
        let policy = self
            .ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Codex permission policy is unavailable".to_owned())?;
        if policy.policy_id != pending.binding.policy_id
            || policy.revision != pending.binding.policy_revision
        {
            return Err("Codex permission policy is stale".into());
        }
        let mut revised = policy.clone();
        match response {
            PermissionDecisionResponseKind::ApproveExactTool => {
                revised
                    .allowed_tools
                    .push(pending.request.tool_name.clone());
                revised.allowed_tools.sort();
                revised.allowed_tools.dedup();
            }
            PermissionDecisionResponseKind::ApproveCategory => {
                revised.allowed_categories.push(pending.request.category);
                revised.allowed_categories.sort();
                revised.allowed_categories.dedup();
            }
            PermissionDecisionResponseKind::Deny | PermissionDecisionResponseKind::ApproveOnce => {
                return Ok(());
            }
        }
        if revised == policy {
            return Ok(());
        }
        revised.revision = policy.revision + 1;
        revised.policy_id = format!(
            "provider-permissions-v{}-{:x}",
            revised.revision,
            Sha256::digest(
                format!(
                    "{}\0{}\0{}",
                    policy.workspace_id, policy.policy_id, revised.revision
                )
                .as_bytes()
            ),
        );
        self.ledger
            .save_policy(&revised)
            .map_err(|error| error.to_string())
    }

    fn settle_receipt(&self, command: &Command, status: ReceiptStatus) -> Result<Receipt, String> {
        let status_name = match status {
            ReceiptStatus::Settled => "settled",
            ReceiptStatus::Unprovable => "unprovable",
            ReceiptStatus::Rejected => "rejected",
            ReceiptStatus::Accepted => "accepted",
        };
        self.ledger
            .settle_receipt(
                &command.idempotency_key,
                status,
                &Event {
                    cursor: 0,
                    event_id: format!("permission-decision-{status_name}:{}", command.receipt_id.0),
                    receipt_id: command.receipt_id.clone(),
                    host_epoch: command.host_epoch,
                    kind: "agentChatPermissionDecisionTerminal".into(),
                    payload: serde_json::json!({"status": status_name}),
                },
            )
            .map_err(|error| error.to_string())
    }
}
