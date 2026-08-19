//! Provider-neutral selection gates for explicitly composed agent-chat authority.

use std::sync::Arc;

use gent_types::AgentChatSelection;

/// Decides whether an authority composition may persist one exact public selection.
///
/// The runtime deliberately does not know a provider catalog. A composition supplies this gate
/// only after it has established which private drivers and modes are safe to start.
pub trait AgentChatSelectionGate: Send + Sync + std::fmt::Debug {
    /// Returns whether this exact provider, model, effort, and mode may be persisted.
    fn allows(&self, selection: &AgentChatSelection) -> bool;
}

impl<T: AgentChatSelectionGate + ?Sized> AgentChatSelectionGate for Arc<T> {
    fn allows(&self, selection: &AgentChatSelection) -> bool {
        (**self).allows(selection)
    }
}

/// Compatibility gate for existing authority compositions that have no private driver allowlist.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllowAnyAgentChatSelection;

impl AgentChatSelectionGate for AllowAnyAgentChatSelection {
    fn allows(&self, _: &AgentChatSelection) -> bool {
        true
    }
}

/// A provider-neutral, exact allowlist supplied by one sealed authority composition.
///
/// A selection is accepted only when all of its public fields match one configured entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExactAgentChatSelectionAllowlist {
    selections: Vec<AgentChatSelection>,
}

impl ExactAgentChatSelectionAllowlist {
    /// Builds a deduplicated exact allowlist.
    #[must_use]
    pub fn new(selections: impl IntoIterator<Item = AgentChatSelection>) -> Self {
        let mut unique = Vec::new();
        for selection in selections {
            if !unique.contains(&selection) {
                unique.push(selection);
            }
        }
        Self { selections: unique }
    }

    /// Returns the configured public selections in their stable composition order.
    #[must_use]
    pub fn selections(&self) -> &[AgentChatSelection] {
        &self.selections
    }
}

impl AgentChatSelectionGate for ExactAgentChatSelectionAllowlist {
    fn allows(&self, selection: &AgentChatSelection) -> bool {
        self.selections.contains(selection)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

    use super::{
        AgentChatSelectionGate, AllowAnyAgentChatSelection, ExactAgentChatSelectionAllowlist,
    };

    fn selection(model: &str) -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: model.into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Ask,
        }
    }

    #[test]
    fn exact_allowlist_matches_every_public_selection_field() {
        let allowed = selection("gpt-5.6");
        let alternative = selection("gpt-5.7");
        let allowlist = ExactAgentChatSelectionAllowlist::new([
            allowed.clone(),
            alternative.clone(),
            allowed.clone(),
        ]);

        assert_eq!(allowlist.selections(), &[allowed.clone(), alternative]);
        assert!(allowlist.allows(&allowed));
        assert!(!allowlist.allows(&selection("gpt-5.8")));
    }

    #[test]
    fn exact_allowlist_rejects_each_nonmatching_public_field() {
        let allowed = selection("gpt-5.6");
        let allowlist = ExactAgentChatSelectionAllowlist::new([allowed.clone()]);
        let mut provider = allowed.clone();
        provider.provider = AgentChatProvider::Claude;
        let mut effort = allowed.clone();
        effort.effort = AgentChatEffort::High;
        let mut mode = allowed.clone();
        mode.mode = AgentChatMode::Plan;

        assert!(!allowlist.allows(&provider));
        assert!(!allowlist.allows(&effort));
        assert!(!allowlist.allows(&mode));
    }

    #[test]
    fn compatibility_and_shared_gates_delegate_without_relaxing_exact_rules() {
        let arbitrary = AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "private-model".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Agent,
        };
        let exact: Arc<dyn AgentChatSelectionGate> = Arc::new(
            ExactAgentChatSelectionAllowlist::new([selection("gpt-5.6")]),
        );

        assert!(AllowAnyAgentChatSelection.allows(&arbitrary));
        assert!(!exact.allows(&arbitrary));
    }
}
