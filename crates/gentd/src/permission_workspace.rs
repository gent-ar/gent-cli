use gent_ports::PolicyLedger;
use gent_types::PolicyRecord;

pub(crate) fn require_supported_launch(
    policy: &PolicyRecord,
    provider: gent_types::AgentChatProvider,
) -> Result<(), gent_ports::PublicProviderRunError> {
    crate::permission_backend_registry::require(provider, policy.containment)
        .map_err(gent_ports::PublicProviderRunError::SandboxUnavailable)
}

pub(crate) fn conversation_containment<L>(
    ledger: &L,
    conversation_id: &str,
) -> Result<gent_types::Containment, gent_ports::LedgerError>
where
    L: gent_ports::AgentChatConversationConfigLedger,
{
    Ok(ledger
        .current_conversation_config(conversation_id)?
        .map(|config| config.containment)
        .unwrap_or_default())
}

pub(crate) fn policy_for_conversation<L>(
    ledger: &L,
    conversation_id: &str,
    workspace_id: &str,
) -> Result<PolicyRecord, gent_ports::LedgerError>
where
    L: PolicyLedger + gent_ports::AgentChatConversationConfigLedger,
{
    Ok(policy_for(ledger, workspace_id)?
        .for_conversation(conversation_containment(ledger, conversation_id)?))
}

pub(crate) fn policy_for<L>(
    ledger: &L,
    workspace_id: &str,
) -> Result<PolicyRecord, gent_ports::LedgerError>
where
    L: PolicyLedger,
{
    ledger.ensure_default_provider_permission_policy(workspace_id)
}

#[cfg(test)]
mod tests {
    use super::policy_for;
    use gent_ports::{PolicyLedger, WorkspaceLedger};
    use gent_runtime::Coordinator;
    use gent_store::SqliteLedger;
    use gent_types::{PermissionMode, PolicyRecord, PolicyScope, WorkspaceRecord};

    #[test]
    fn a_workspace_policy_revision_gates_its_next_request() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            })
            .unwrap();
        let initial = policy_for(&ledger, "workspace-1").unwrap();
        assert_eq!(
            (initial.revision, initial.mode),
            (1, PermissionMode::AskEveryTime)
        );
        ledger
            .save_policy(&PolicyRecord {
                policy_id: "workspace-revision-2".into(),
                workspace_id: "workspace-1".into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 2,
                mode: PermissionMode::Autonomous,
                containment: gent_types::Containment::Sandboxed,
                allowed_tools: Vec::new(),
                allowed_categories: Vec::new(),
            })
            .unwrap();
        let current = policy_for(&ledger, "workspace-1").unwrap();
        assert_eq!(
            (current.revision, current.mode),
            (2, PermissionMode::Autonomous)
        );
    }

    #[test]
    fn persisted_broad_modes_fail_before_provider_configuration_or_spawn() {
        for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
            let policy = PolicyRecord {
                policy_id: "persisted".into(),
                workspace_id: "workspace-1".into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 4,
                mode,
                containment: gent_types::Containment::Sandboxed,
                allowed_tools: vec!["Bash".into()],
                allowed_categories: Vec::new(),
            };
            assert!(
                super::require_supported_launch(&policy, gent_types::AgentChatProvider::Claude)
                    .is_err()
            );
        }
    }

    fn prepared_broad_policy(
        selection: gent_types::AgentChatSelection,
        mode: PermissionMode,
    ) -> SqliteLedger {
        use gent_ports::AgentChatWorkspaceLedger;
        use gent_types::{
            AgentChatConversationCreate, AgentChatConversationId, AgentChatRunId, HostEpoch,
            ReceiptId,
        };
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = AgentChatConversationId("conversation-a".into());
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("create".into()),
                    idempotency_key: "create".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    run_id: AgentChatRunId("run-a".into()),
                    selection,
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-a".into(),
                    canonical_path: "/workspace-a".into(),
                },
            )
            .unwrap();
        ledger
            .save_policy(&PolicyRecord {
                policy_id: "stored-broad-policy".into(),
                workspace_id: "workspace-a".into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 1,
                mode,
                containment: gent_types::Containment::Sandboxed,
                allowed_tools: Vec::new(),
                allowed_categories: Vec::new(),
            })
            .unwrap();
        gent_ports::AgentChatConversationConfigLedger::save_conversation_config(
            &ledger,
            &gent_types::AgentChatConversationConfigRecord {
                conversation_id: conversation.clone(),
                revision: 1,
                system_prompt: None,
                append_system_prompt: false,
                max_turns: None,
                thinking_enabled: None,
                disallowed_tools: Vec::new(),
                containment: gent_types::Containment::Sandboxed,
            },
        )
        .unwrap();
        crate::claude_prompt_lifecycle_tests::prompt(&ledger, &conversation, "a");
        ledger
    }

    #[test]
    fn a_sandboxed_conversation_without_containment_never_reaches_the_claude_launch_or_resume_port()
    {
        use crate::claude_prompt_lifecycle_tests::{Resolver, Runner, compatibility, profile};
        use crate::public_driver_runtime::PublicDriversRuntime;
        use gent_types::{
            AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, CapabilitySet,
            HostEpoch,
        };
        for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
            let ledger = prepared_broad_policy(
                AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: "claude-test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
                mode,
            );
            let runner = Runner::default();
            let compatibility = compatibility();
            let runtime = PublicDriversRuntime::new(
                profile(&compatibility),
                Coordinator::new(ledger.clone(), CapabilitySet::default()),
                ledger,
                compatibility,
                runner.clone(),
                Resolver,
            )
            .unwrap();
            let mut host = crate::approved_claude_host::ApprovedClaudeHost::new(
                runtime,
                "daemon-a".into(),
                HostEpoch(1),
                1,
                None,
            );
            assert!(matches!(
                host.tick().unwrap().dispatch,
                Some(
                    crate::claude_prompt_lifecycle::ClaudePromptDispatchOutcome::Unprovable { .. }
                )
            ));
            let state = runner.0.lock().unwrap();
            assert_eq!(state.starts + state.resumes, 0);
            assert!(state.prepared_goals.is_empty());
        }
    }

    #[test]
    fn a_sandboxed_conversation_without_containment_never_reaches_the_codex_launch_or_resume_port()
    {
        use crate::codex_prompt_lifecycle_tests::{
            Resolver, Runner, compatibility, profile, selection,
        };
        use crate::public_driver_runtime::PublicDriversRuntime;
        use gent_types::{CapabilitySet, HostEpoch};
        for mode in [PermissionMode::Autonomous, PermissionMode::Bypass] {
            let ledger = prepared_broad_policy(selection(), mode);
            let runner = Runner::default();
            let compatibility = compatibility();
            let runtime = PublicDriversRuntime::new(
                profile(&compatibility),
                Coordinator::new(ledger.clone(), CapabilitySet::default()),
                ledger,
                compatibility,
                runner.clone(),
                Resolver,
            )
            .unwrap();
            let mut host = crate::approved_codex_host::ApprovedCodexHost::new(
                runtime,
                "daemon-a".into(),
                HostEpoch(1),
                1,
            );
            assert!(matches!(
                host.tick().unwrap().dispatch,
                Some(crate::codex_prompt_lifecycle::CodexPromptDispatchOutcome::Unprovable { .. })
            ));
            let state = runner.state.lock().unwrap();
            assert_eq!(state.starts + state.resumes, 0);
            assert!(state.pending.is_none());
        }
    }
}
