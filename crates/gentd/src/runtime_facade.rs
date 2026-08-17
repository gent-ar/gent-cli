//! Daemon composition of observer and explicitly approved durable-chat services.

use gent_drivers::installer::SystemDependencyInstaller;
use gent_protocol::AGENT_CHAT_INTENTS_CAPABILITY;
use gent_runtime::catalog::validate_observed_capabilities;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationService, AgentChatPromptAuthority,
    AgentChatPromptService, AgentChatReadService, AgentChatSelectionSwitchAuthority,
    AgentChatSelectionSwitchService, AttachmentService, Coordinator, DependencyActionService,
    RuntimeMaintenanceAuthority, RuntimeMaintenanceService,
};
use gent_store::{FileAttachmentBlobs, SqliteLedger};
use gent_types::CapabilitySet;

use crate::compatibility_assessment::CompatibilityAssessment;
use crate::dependency_actions::SystemDependencyExecutor;
use crate::dependency_catalog::DependencyCatalog;
use crate::node_runtime;
use crate::public_runs::{DaemonPublicRuns, observer_service};
use crate::runtime_update_config::DaemonRuntimeUpdateChecks;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeFacade {
    agent_chat_conversations: AgentChatConversationService<SqliteLedger>,
    agent_chat_prompts: AgentChatPromptService<SqliteLedger>,
    agent_chat_switches: AgentChatSelectionSwitchService<SqliteLedger>,
    agent_chat_reads: Option<AgentChatReadService<SqliteLedger>>,
    runtime_maintenance: RuntimeMaintenanceService<SqliteLedger>,
    attachments: AttachmentService<SqliteLedger, FileAttachmentBlobs>,
    coordinator: Coordinator<SqliteLedger>,
    dependencies: DependencyCatalog,
    dependency_actions:
        DependencyActionService<SqliteLedger, SystemDependencyExecutor<SystemDependencyInstaller>>,
    public_runs: DaemonPublicRuns,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
}

/// Builds the current observer-only daemon composition.
///
/// # Errors
/// Returns an error when capabilities drift or required local storage cannot open.
#[cfg(test)]
pub(crate) fn build_runtime(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    build_runtime_with_update_checks(data_dir, observed_capabilities, compatibility, None)
}

/// Builds the daemon with explicitly configured read-only update checks and durable-chat reads.
///
/// # Errors
/// Returns an error when capabilities drift or required local storage cannot open.
pub(crate) fn build_runtime_with_update_checks(
    data_dir: &std::path::Path,
    observed_capabilities: &CapabilitySet,
    compatibility: CompatibilityAssessment,
    runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
) -> Result<RuntimeFacade, Box<dyn std::error::Error>> {
    let capabilities = validate_observed_capabilities(observed_capabilities)?;
    let agent_chat_enabled = capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY);
    let maintenance_enabled = capabilities
        .0
        .iter()
        .any(|capability| capability == gent_protocol::RUNTIME_MAINTENANCE_CAPABILITY);
    let ledger = SqliteLedger::open(data_dir.join("gent.db"))?;
    crate::permission_workspace::ensure(&ledger, data_dir)?;
    let attachments = AttachmentService::new(
        ledger.clone(),
        FileAttachmentBlobs::open(data_dir.join("attachments"))?,
    );
    let coordinator = Coordinator::new(ledger.clone(), capabilities);
    coordinator.persist_capability_catalog()?;
    Ok(RuntimeFacade {
        agent_chat_conversations: AgentChatConversationService::new(
            ledger.clone(),
            chat_authority(agent_chat_enabled),
        ),
        agent_chat_prompts: AgentChatPromptService::new(
            ledger.clone(),
            prompt_authority(agent_chat_enabled),
        ),
        agent_chat_switches: AgentChatSelectionSwitchService::new(
            ledger.clone(),
            switch_authority(agent_chat_enabled),
        ),
        agent_chat_reads: agent_chat_enabled.then(|| AgentChatReadService::new(ledger.clone())),
        runtime_maintenance: RuntimeMaintenanceService::new(
            ledger.clone(),
            maintenance_authority(maintenance_enabled),
        ),
        public_runs: observer_service(coordinator.clone(), compatibility.clone()),
        runtime_update_checks,
        attachments,
        coordinator,
        dependencies: DependencyCatalog::with_private_prefix(
            compatibility,
            data_dir.join("providers").join("npm-global"),
        ),
        dependency_actions: DependencyActionService::new(
            ledger,
            SystemDependencyExecutor::new(
                SystemDependencyInstaller,
                node_runtime::private_npm_prefix(data_dir),
            ),
        ),
    })
}

fn chat_authority(enabled: bool) -> AgentChatConversationAuthority {
    if enabled {
        AgentChatConversationAuthority::Approved
    } else {
        AgentChatConversationAuthority::Observer
    }
}

fn prompt_authority(enabled: bool) -> AgentChatPromptAuthority {
    if enabled {
        AgentChatPromptAuthority::Approved
    } else {
        AgentChatPromptAuthority::Observer
    }
}

fn switch_authority(enabled: bool) -> AgentChatSelectionSwitchAuthority {
    if enabled {
        AgentChatSelectionSwitchAuthority::Approved
    } else {
        AgentChatSelectionSwitchAuthority::Observer
    }
}

fn maintenance_authority(enabled: bool) -> RuntimeMaintenanceAuthority {
    if enabled {
        RuntimeMaintenanceAuthority::Approved
    } else {
        RuntimeMaintenanceAuthority::Observer
    }
}

include!("runtime_facade_api.rs");
