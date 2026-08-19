//! Explicit non-observer facade constructors kept outside the default composition.

use std::sync::Arc;

use gent_runtime::ExactAgentChatSelectionAllowlist;

use super::{DaemonCompositionState, RuntimeFacade};
use crate::{
    ordinary_authority_composition::OrdinaryAuthorityRuntime,
    runtime_update_config::DaemonRuntimeUpdateChecks,
};

impl RuntimeFacade {
    /// Builds an explicit future authority seam for exact, read-only turn following.
    ///
    /// No shipped bootstrap calls this constructor or advertises the corresponding capability.
    ///
    /// # Errors
    /// Returns an error when the durable attachment store cannot open.
    #[allow(dead_code)] // Reserved for an explicit future authority composition only.
    pub(crate) fn from_state_with_turn_follow_authority(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::from_state_inner(
            state,
            runtime_update_checks,
            true,
            None,
            Arc::new(gent_runtime::AllowAnyAgentChatSelection),
        )
    }

    /// Builds the dormant ordinary terminal seam with its one private lifecycle router.
    ///
    /// The caller must have already validated the authority profile, evidence, private prefix,
    /// and canonical workspace bindings. This constructor is deliberately absent from bootstrap.
    ///
    /// # Errors
    /// Returns an error when the durable attachment store cannot open.
    #[allow(dead_code)] // Reserved for the explicit ordinary authority composition.
    pub(crate) fn from_state_with_ordinary_terminal_authority(
        state: DaemonCompositionState,
        runtime_update_checks: Option<DaemonRuntimeUpdateChecks>,
        authority: &OrdinaryAuthorityRuntime,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::from_state_inner(
            state,
            runtime_update_checks,
            true,
            Some(authority.router()),
            Arc::new(ExactAgentChatSelectionAllowlist::new(
                authority.selections().iter().cloned(),
            )),
        )
    }
}
