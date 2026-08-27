//! Dormant, separately approved public-driver composition edge; `main` never constructs it.
use crate::provider_effects::ProviderEffectDispatcher;
use gent_ports::{
    ActiveGoalResolver, AgentChatPromptDispatchLedger, AgentChatReadLedger,
    ConversationActivityLedger, Ledger, PublicProviderResolver, PublicProviderRunner,
    TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchResult, AgentChatPromptDispatchService, AgentChatReadService,
    AgentChatTranscriptIngress, ProviderActivityIngress, PublicRunService, RuntimeError,
};
use gent_types::{AgentChatProvider, AgentChatSelection, GoalProjection, HostEpoch};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) use composition::DriverCompatibilityAuthorizer;
pub(crate) use session::NormalizedSessionFact;
pub(crate) use types::{PublicDriverFact, PublicDriverFactResult, PublicDriversRuntimeError};

/// A fully injected, authority-gated runtime deliberately absent from `RuntimeFacade`.
#[derive(Debug)]
pub(crate) struct PublicDriversRuntime<L, D, R> {
    ledger: L,
    runs: Arc<PublicRunService<L, D, DriverCompatibilityAuthorizer, R>>,
    runner: D,
    effects: ProviderEffectDispatcher<L>,
    activity: ProviderActivityIngress<L>,
    transcripts: AgentChatTranscriptIngress<L>,
    dispatches: AgentChatPromptDispatchService<L>,
    reads: AgentChatReadService<L>,
    pub(crate) contexts: context::RunContextProjection<L>,
    goal_resolver: Option<Arc<dyn ActiveGoalResolver>>,
    attachment_root: Option<PathBuf>,
    codex_attachment_root: Option<PathBuf>,
}
impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: PublicProviderRunner + Clone,
    R: PublicProviderResolver,
{
    #[must_use]
    pub(crate) fn with_active_goal_resolver(
        mut self,
        resolver: Arc<dyn ActiveGoalResolver>,
    ) -> Self {
        self.goal_resolver = Some(resolver);
        self
    }

    pub(crate) fn with_attachment_roots(
        mut self,
        attachment_root: PathBuf,
        codex_attachment_root: PathBuf,
    ) -> Self {
        self.attachment_root = Some(attachment_root);
        self.codex_attachment_root = Some(codex_attachment_root);
        self
    }

    /// Resolves fresh goal context immediately before a daemon-owned provider turn.
    pub(crate) fn active_goal_for(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<GoalProjection>, RuntimeError> {
        self.goal_resolver.as_ref().map_or(Ok(None), |resolver| {
            resolver
                .resolve_active_goal(conversation_id, run_id)
                .map_err(RuntimeError::from)
        })
    }

    /// Returns the only process lifecycle service constructed by this authority profile.
    #[must_use]
    pub(crate) fn runs(&self) -> &Arc<PublicRunService<L, D, DriverCompatibilityAuthorizer, R>> {
        &self.runs
    }

    /// Returns the exact clone shared with the durable public-run service.
    #[must_use]
    pub(crate) fn runner(&self) -> D {
        self.runner.clone()
    }

    #[must_use]
    pub(crate) fn ledger(&self) -> L {
        self.ledger.clone()
    }

    pub(crate) fn attachment_roots(&self) -> Result<(PathBuf, PathBuf), RuntimeError> {
        self.attachment_root
            .clone()
            .zip(self.codex_attachment_root.clone())
            .ok_or_else(|| {
                gent_ports::PublicProviderRunError::Failed(
                    "provider attachment delivery is unavailable".into(),
                )
                .into()
            })
    }

    /// Persists one runner-owned source fact through its matching durable ingress.
    pub(crate) fn record(
        &self,
        run_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        fact: PublicDriverFact,
    ) -> Result<PublicDriverFactResult, RuntimeError> {
        match fact {
            PublicDriverFact::SessionEffect { event_id, effect } => self
                .effects
                .record(event_id, run_id, coordinator_id, host_epoch, &effect)
                .map(PublicDriverFactResult::Lifecycle),
            PublicDriverFact::PublicWire { event_id, fact } => self
                .effects
                .record_public_wire_fact(event_id, run_id, coordinator_id, host_epoch, &fact)
                .map(PublicDriverFactResult::Lifecycle),
            PublicDriverFact::Activity(activity) => self
                .activity
                .record(coordinator_id, activity)
                .map(PublicDriverFactResult::Activity),
            PublicDriverFact::Transcript(append) => self
                .transcripts
                .append(&append)
                .map(PublicDriverFactResult::Transcript),
        }
    }

    /// Claims the next durable prompt only for this approved daemon lifecycle owner.
    ///
    /// # Errors
    /// Returns an error when the durable ownership fence rejects the claim.
    pub(crate) fn claim_prompt(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        self.dispatches.claim(coordinator_id, host_epoch, provider)
    }

    /// Marks the durable boundary before a provider process may be launched.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects the release.
    pub(crate) fn begin_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .begin_launch(message_id, coordinator_id, host_epoch)
    }

    /// Confirms that the daemon-owned runner successfully launched the provider process.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects settlement.
    pub(crate) fn confirm_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .confirm_started(message_id, coordinator_id, host_epoch)
    }

    /// Returns a claim to the durable outbox before a launch boundary is crossed.
    pub(crate) fn release_prompt_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_claim(message_id, coordinator_id, host_epoch)
    }

    /// Returns a launch marker only after a local result proves no provider runner was called.
    pub(crate) fn release_unstarted_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_unstarted_launch(message_id, coordinator_id, host_epoch)
    }

    /// Retires an ambiguous launch without allowing the durable prompt to replay automatically.
    pub(crate) fn mark_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .mark_unprovable(message_id, coordinator_id, host_epoch)
    }

    /// Settles one terminal provider prompt through its durable outbox owner.
    pub(crate) fn settle_prompt(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .settle(message_id, coordinator_id, host_epoch)
    }

    pub(crate) fn settle_prompt_terminal(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        phase: gent_types::DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .settle_terminal(message_id, coordinator_id, host_epoch, phase)
    }

    /// Recovers only pre-launch work after a successor daemon has fenced the previous epoch.
    pub(crate) fn recover_prompts(&self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        self.dispatches.recover(host_epoch)
    }
}

impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: gent_ports::ToolSourceLedger + gent_ports::AgentChatWorkspaceLedger,
{
    pub(crate) fn validate_tool_sources_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
        source_ids: &[String],
    ) -> Result<Vec<gent_types::ToolSourceRecord>, RuntimeError> {
        let workspace = self.workspace_for_run(conversation_id, run_id)?;
        source_ids
            .iter()
            .map(|source_id| {
                let source = self.ledger.find_tool_source(source_id)?.ok_or_else(|| {
                    gent_ports::LedgerError::Invariant(
                        "selected MCP tool source does not exist".into(),
                    )
                })?;
                if source.workspace_id != workspace.workspace_id
                    || source.kind != gent_types::ToolSourceKind::McpServer
                {
                    return Err(gent_ports::LedgerError::Invariant(
                        "selected tool source is not available in this workspace".into(),
                    )
                    .into());
                }
                Ok(source)
            })
            .collect()
    }
}

impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: gent_ports::AgentChatWorkspaceLedger,
{
    /// Resolves the one daemon-canonical workspace bound to the exact durable run.
    pub(crate) fn workspace_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<gent_types::WorkspaceRecord, RuntimeError> {
        self.ledger
            .agent_chat_workspace_for_run(conversation_id, run_id)
            .map_err(RuntimeError::from)
    }
}
impl<L: AgentChatReadLedger, D, R> PublicDriversRuntime<L, D, R> {
    pub(crate) fn selection_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<AgentChatSelection, RuntimeError> {
        self.reads.run_selection(conversation_id, run_id)
    }
}

#[path = "public_driver_runtime_composition.rs"]
mod composition;
mod context;
pub(crate) mod session;
mod types;
