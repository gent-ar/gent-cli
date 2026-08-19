//! Explicit input gate for independently evidence-bound dormant provider authority.
//!
//! This is deliberately not a daemon argument surface or composition entry point. It only
//! validates supplied values without reading a record, loading compatibility, or creating a
//! provider host. A future reviewed bootstrap must derive its coordinator and host epoch from its
//! opened daemon state; neither is accepted here.

use std::path::PathBuf;

/// Raw inputs a future private bootstrap may receive from its own reviewed configuration source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OrdinaryAuthorityBootstrapInput {
    pub(crate) enabled: bool,
    pub(crate) agent_chat_authority: bool,
    pub(crate) codex_evidence_record: Option<PathBuf>,
    pub(crate) codex_trusted_keys: Vec<String>,
    pub(crate) claude_evidence_record: Option<PathBuf>,
    pub(crate) claude_trusted_keys: Vec<String>,
    pub(crate) compatibility_cache: Option<PathBuf>,
    pub(crate) compatibility_keys: Vec<String>,
}

/// One independently evidence-bound public provider selected for private preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OrdinaryProviderBootstrapConfig {
    Claude {
        evidence_record: PathBuf,
        trusted_keys: Vec<String>,
    },
    Codex {
        evidence_record: PathBuf,
        trusted_keys: Vec<String>,
    },
}

/// Complete, non-secret material required before a private bootstrap can preflight authority.
///
/// The paths and keys are retained exactly as provided; cryptographic verification and file I/O
/// remain the responsibility of the later composition edge. This value intentionally has no
/// coordinator identity or host epoch fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrdinaryAuthorityBootstrapConfig {
    pub(crate) providers: Vec<OrdinaryProviderBootstrapConfig>,
    pub(crate) compatibility_cache: PathBuf,
    pub(crate) compatibility_keys: Vec<String>,
}

/// Controlled rejection before any authority evidence or compatibility input is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityBootstrapError {
    #[error("ordinary provider authority conflicts with durable chat-only authority")]
    ConflictsWithAgentChatAuthority,
    #[error("ordinary provider authority settings require explicit authority opt-in")]
    SettingsRequireOptIn,
    #[error("ordinary provider authority requires at least one selected provider")]
    MissingProvider,
    #[error("ordinary provider authority requires a Codex evidence record")]
    MissingCodexEvidence,
    #[error("ordinary provider authority requires at least one Codex evidence key")]
    MissingCodexKeys,
    #[error("ordinary provider authority requires a Claude evidence record")]
    MissingClaudeEvidence,
    #[error("ordinary provider authority requires at least one Claude evidence key")]
    MissingClaudeKeys,
    #[error("ordinary provider authority requires a signed compatibility cache")]
    MissingCompatibilityCache,
    #[error("ordinary provider authority requires at least one compatibility key")]
    MissingCompatibilityKeys,
}

/// Validates an explicit authority opt-in without performing any filesystem access.
///
/// `None` is the only observer result. Any ordinary-authority setting without `enabled` is
/// rejected, keeping the ordinary observer startup free from evidence reads and hidden authority.
pub(crate) fn parse(
    input: OrdinaryAuthorityBootstrapInput,
) -> Result<Option<OrdinaryAuthorityBootstrapConfig>, OrdinaryAuthorityBootstrapError> {
    if !input.enabled {
        return if has_settings(&input) {
            Err(OrdinaryAuthorityBootstrapError::SettingsRequireOptIn)
        } else {
            Ok(None)
        };
    }
    if input.agent_chat_authority {
        return Err(OrdinaryAuthorityBootstrapError::ConflictsWithAgentChatAuthority);
    }
    let mut providers = Vec::new();
    select_provider(
        &mut providers,
        input.codex_evidence_record,
        input.codex_trusted_keys,
        OrdinaryAuthorityBootstrapError::MissingCodexEvidence,
        OrdinaryAuthorityBootstrapError::MissingCodexKeys,
        |evidence_record, trusted_keys| OrdinaryProviderBootstrapConfig::Codex {
            evidence_record,
            trusted_keys,
        },
    )?;
    select_provider(
        &mut providers,
        input.claude_evidence_record,
        input.claude_trusted_keys,
        OrdinaryAuthorityBootstrapError::MissingClaudeEvidence,
        OrdinaryAuthorityBootstrapError::MissingClaudeKeys,
        |evidence_record, trusted_keys| OrdinaryProviderBootstrapConfig::Claude {
            evidence_record,
            trusted_keys,
        },
    )?;
    if providers.is_empty() {
        return Err(OrdinaryAuthorityBootstrapError::MissingProvider);
    }
    let compatibility_cache = input
        .compatibility_cache
        .ok_or(OrdinaryAuthorityBootstrapError::MissingCompatibilityCache)?;
    require_keys(
        &input.compatibility_keys,
        OrdinaryAuthorityBootstrapError::MissingCompatibilityKeys,
    )?;
    Ok(Some(OrdinaryAuthorityBootstrapConfig {
        providers,
        compatibility_cache,
        compatibility_keys: input.compatibility_keys,
    }))
}

fn select_provider<F>(
    providers: &mut Vec<OrdinaryProviderBootstrapConfig>,
    evidence_record: Option<PathBuf>,
    trusted_keys: Vec<String>,
    missing_evidence: OrdinaryAuthorityBootstrapError,
    missing_keys: OrdinaryAuthorityBootstrapError,
    provider: F,
) -> Result<(), OrdinaryAuthorityBootstrapError>
where
    F: FnOnce(PathBuf, Vec<String>) -> OrdinaryProviderBootstrapConfig,
{
    match (evidence_record, trusted_keys.is_empty()) {
        (None, true) => Ok(()),
        (None, false) => Err(missing_evidence),
        (Some(_), true) => Err(missing_keys),
        (Some(evidence_record), false) => {
            providers.push(provider(evidence_record, trusted_keys));
            Ok(())
        }
    }
}

fn has_settings(input: &OrdinaryAuthorityBootstrapInput) -> bool {
    input.codex_evidence_record.is_some()
        || !input.codex_trusted_keys.is_empty()
        || input.claude_evidence_record.is_some()
        || !input.claude_trusted_keys.is_empty()
        || input.compatibility_cache.is_some()
        || !input.compatibility_keys.is_empty()
}

fn require_keys(
    keys: &[String],
    error: OrdinaryAuthorityBootstrapError,
) -> Result<(), OrdinaryAuthorityBootstrapError> {
    (!keys.is_empty()).then_some(()).ok_or(error)
}

#[cfg(test)]
#[path = "ordinary_authority_bootstrap_tests.rs"]
mod tests;
