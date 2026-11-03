use std::path::Path;

use gent_ports::{AgentChatReadLedger, AgentChatWorkspaceLedger, Ledger, PolicyLedger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatProvider, AgentChatSelection, HostEpoch, PolicyRecord, PolicyScope, RunVersionLock,
    WorkspaceRecord,
};

use super::EgressError;

#[derive(Debug)]
pub(crate) struct RunFence {
    pub ledger: SqliteLedger,
    pub coordinator: Coordinator<SqliteLedger>,
    pub conversation_id: String,
    pub run_id: String,
    pub workspace: WorkspaceRecord,
    pub selection: AgentChatSelection,
    pub policy: PolicyRecord,
    pub host_epoch: HostEpoch,
    pub lock: RunVersionLock,
}

impl RunFence {
    pub fn capture(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
    ) -> Result<Self, EgressError> {
        let detail = ledger
            .read_agent_chat_detail(&conversation_id)
            .map_err(|_| EgressError::Configuration)?;
        let run = detail
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .ok_or(EgressError::Configuration)?;
        if detail.current_run_id != run_id
            || !matches!(
                (run.selection.provider, lock.provider.as_str()),
                (AgentChatProvider::Claude, "claude")
                    | (AgentChatProvider::Codex, "codex")
                    | (AgentChatProvider::Claurst, "claurst")
            )
        {
            return Err(EgressError::Configuration);
        }
        let workspace = ledger
            .agent_chat_workspace_for_run(&conversation_id, &run_id)
            .map_err(|_| EgressError::Configuration)?;
        if std::fs::canonicalize(&workspace.canonical_path)
            .ok()
            .as_deref()
            != Some(Path::new(&workspace.canonical_path))
        {
            return Err(EgressError::Configuration);
        }
        let policy = ledger
            .current_policy(&workspace.workspace_id, PolicyScope::ProviderPermissions)
            .map_err(|_| EgressError::Configuration)?
            .ok_or(EgressError::Configuration)?;
        let host_epoch = coordinator
            .status()
            .map_err(|_| EgressError::Configuration)?
            .host_epoch;
        let ingress = ledger
            .host_ingress()
            .map_err(|_| EgressError::Configuration)?;
        if ingress.epoch != host_epoch || ingress.mode != gent_ports::IngressMode::Open {
            return Err(EgressError::Configuration);
        }
        Ok(Self {
            ledger,
            coordinator,
            conversation_id,
            run_id,
            workspace,
            selection: run.selection.clone(),
            policy,
            host_epoch,
            lock,
        })
    }

    pub fn current(&self) -> bool {
        let Ok(epoch) = self.coordinator.status() else {
            return false;
        };
        let Ok(detail) = self.ledger.read_agent_chat_detail(&self.conversation_id) else {
            return false;
        };
        let Ok(workspace) = self
            .ledger
            .agent_chat_workspace_for_run(&self.conversation_id, &self.run_id)
        else {
            return false;
        };
        let Ok(policy) = self.ledger.current_policy(
            &self.workspace.workspace_id,
            PolicyScope::ProviderPermissions,
        ) else {
            return false;
        };
        epoch.host_epoch == self.host_epoch
            && self.ledger.host_ingress().ok().is_some_and(|ingress| {
                ingress.epoch == self.host_epoch && ingress.mode == gent_ports::IngressMode::Open
            })
            && workspace == self.workspace
            && policy.as_ref() == Some(&self.policy)
            && detail.current_run_id == self.run_id
            && detail
                .runs
                .iter()
                .any(|run| run.run_id == self.run_id && run.selection == self.selection)
    }

    pub fn matches(&self, launch: &gent_drivers::supervisor::ProviderLaunch) -> bool {
        self.current()
            && launch.run_id.as_deref() == Some(&self.run_id)
            && launch.lock == self.lock
            && launch.provider == self.lock.provider
            && launch.executable == Path::new(&self.lock.canonical_path)
            && launch.workspace_root.as_deref() == Some(Path::new(&self.workspace.canonical_path))
            && launch.workspace_access
                == gent_types::SandboxWorkspaceAccess::from_mode(self.selection.mode)
    }
}
