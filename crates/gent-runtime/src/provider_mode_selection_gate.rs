//! Composition-owned selection gate for providers whose models evolve independently.

use gent_types::{AgentChatMode, AgentChatProvider, AgentChatSelection};

use crate::AgentChatSelectionGate;

/// Allows only configured providers and modes while accepting any valid model and effort.
///
/// Provider executable compatibility is checked at launch against its immutable lock. Keeping
/// model names out of the daemon bootstrap allowlist lets supported vendors add models without
/// turning a stale local configuration into a second source of truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderModeSelectionGate {
    providers: Vec<AgentChatProvider>,
    modes: Vec<AgentChatMode>,
}

impl ProviderModeSelectionGate {
    /// Creates a stable deduplicated provider/mode gate.
    #[must_use]
    pub fn new(
        providers: impl IntoIterator<Item = AgentChatProvider>,
        modes: impl IntoIterator<Item = AgentChatMode>,
    ) -> Self {
        Self {
            providers: unique(providers),
            modes: unique(modes),
        }
    }
}

impl AgentChatSelectionGate for ProviderModeSelectionGate {
    fn allows(&self, selection: &AgentChatSelection) -> bool {
        selection.validate().is_ok()
            && self.providers.contains(&selection.provider)
            && self.modes.contains(&selection.mode)
    }
}

fn unique<T: Eq>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

    use super::*;

    fn selection(model: &str) -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: model.into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Ask,
        }
    }

    #[test]
    fn accepts_new_valid_models_and_efforts_for_the_composed_provider_mode() {
        let gate = ProviderModeSelectionGate::new(
            [AgentChatProvider::Claude, AgentChatProvider::Codex],
            [AgentChatMode::Ask, AgentChatMode::Plan],
        );
        let mut high_effort = selection("gpt-5.7");
        high_effort.effort = AgentChatEffort::High;

        assert!(gate.allows(&selection("gpt-5.6")));
        assert!(gate.allows(&high_effort));
    }

    #[test]
    fn denies_uncomposed_provider_mode_or_invalid_model() {
        let gate = ProviderModeSelectionGate::new([AgentChatProvider::Codex], [AgentChatMode::Ask]);
        let mut claude = selection("sonnet");
        claude.provider = AgentChatProvider::Claude;
        let mut agent = selection("gpt-5.6");
        agent.mode = AgentChatMode::Agent;

        assert!(!gate.allows(&claude));
        assert!(!gate.allows(&agent));
        assert!(!gate.allows(&selection(" ")));
    }
}
