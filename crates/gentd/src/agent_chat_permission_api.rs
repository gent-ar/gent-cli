use async_trait::async_trait;
use gent_ports::{AgentChatReadLedger, PendingPermissionLedger};
use gent_protocol::AgentChatPermissionFrame;
use gent_store::SqliteLedger;
use gent_types::PermissionDecisionResponse;

use crate::ordinary_lifecycle_cadence::OrdinaryPromptIngress;

#[path = "agent_chat_permission_api_decision.rs"]
mod decision;
#[path = "agent_chat_permission_api_receipt.rs"]
pub(crate) mod receipt;

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
}

#[cfg(test)]
#[path = "agent_chat_permission_api_tests.rs"]
mod tests;
