//! A policy revision cannot silently replace an active worker's containment binding.
use gent_ports::{AgentChatWorkspaceLedger, ConversationLedger, PolicyLedger};
use gent_protocol::PermissionPolicyFrame;
use gent_types::PolicyScope;

pub(crate) fn require_idle_transition<L>(
    ledger: &L,
    frame: &PermissionPolicyFrame,
) -> Result<(), String>
where
    L: PolicyLedger + ConversationLedger + AgentChatWorkspaceLedger,
{
    let PermissionPolicyFrame::Save { policy, .. } = frame else {
        return Ok(());
    };
    let current = ledger
        .current_policy(&policy.workspace_id, PolicyScope::ProviderPermissions)
        .map_err(|error| error.to_string())?;
    if current.as_ref() == Some(policy) {
        return Ok(());
    }
    if !policy.mode.is_unattended()
        && !current
            .as_ref()
            .is_some_and(|record| record.mode.is_unattended())
    {
        return Ok(());
    }
    for conversation in ledger
        .list_conversations()
        .map_err(|error| error.to_string())?
    {
        for run in ledger
            .list_conversation_runs(&conversation.conversation_id)
            .map_err(|error| error.to_string())?
        {
            let turns = ledger
                .list_run_turns(&run.run_id)
                .map_err(|error| error.to_string())?;
            if !turns.iter().any(|turn| !turn.phase.is_terminal()) {
                continue;
            }
            let workspace = ledger
                .agent_chat_workspace_for_run(&conversation.conversation_id, &run.run_id)
                .map_err(|error| error.to_string())?;
            if workspace.workspace_id == policy.workspace_id {
                return Err("Finish or stop active work in this workspace before changing its contained permission mode. The current mode remains in effect.".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gent_store::SqliteLedger;
    use gent_types::*;

    fn fixture() -> (SqliteLedger, PolicyRecord) {
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
                        provider: AgentChatProvider::Claurst,
                        model: "model".into(),
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
        let policy = PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::AskEveryTime,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        };
        ledger.save_policy(&policy).unwrap();
        ledger
            .create_turn(&TurnRecord {
                turn_id: "turn".into(),
                conversation_id: "conversation".into(),
                run_id: "run".into(),
                sequence: 1,
                phase: DurableTurnPhase::Active,
            })
            .unwrap();
        (ledger, policy)
    }
    fn frame(policy: PolicyRecord) -> PermissionPolicyFrame {
        PermissionPolicyFrame::Save {
            request_id: "save".into(),
            policy,
            bypass_consent: true,
            expected_provider: Some(AgentChatProvider::Claurst),
        }
    }
    #[test]
    fn active_work_cannot_be_silently_upgraded_to_an_unbound_worker() {
        let (ledger, current) = fixture();
        for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
            let mut changed = current.clone();
            changed.revision += 1;
            changed.mode = mode;
            assert!(
                require_idle_transition(&ledger, &frame(changed))
                    .unwrap_err()
                    .contains("current mode remains")
            );
            assert_eq!(
                ledger
                    .current_policy("workspace", PolicyScope::ProviderPermissions)
                    .unwrap(),
                Some(current.clone())
            );
        }
        ledger
            .replace_turn_phase(
                "turn",
                DurableTurnPhase::Active,
                DurableTurnPhase::Completed,
            )
            .unwrap();
        let mut changed = current;
        changed.revision += 1;
        changed.mode = PermissionMode::Autonomous;
        assert!(require_idle_transition(&ledger, &frame(changed)).is_ok());
    }
    #[test]
    fn exact_revalidation_preserves_the_binding_but_a_new_revision_requires_idle() {
        let (ledger, mut current) = fixture();
        current.policy_id = "broad".into();
        current.revision += 1;
        current.mode = PermissionMode::Bypass;
        ledger.save_policy(&current).unwrap();
        assert!(require_idle_transition(&ledger, &frame(current.clone())).is_ok());
        current.revision += 1;
        assert!(require_idle_transition(&ledger, &frame(current)).is_err());
    }
}
