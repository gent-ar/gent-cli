//! Explicit, all-or-nothing input gate for the dormant ordinary provider authority.
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

/// Complete, non-secret material required before a private bootstrap can preflight authority.
///
/// The paths and keys are retained exactly as provided; cryptographic verification and file I/O
/// remain the responsibility of the later composition edge. This value intentionally has no
/// coordinator identity or host epoch fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrdinaryAuthorityBootstrapConfig {
    pub(crate) codex_evidence_record: PathBuf,
    pub(crate) codex_trusted_keys: Vec<String>,
    pub(crate) claude_evidence_record: PathBuf,
    pub(crate) claude_trusted_keys: Vec<String>,
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
    let codex_evidence_record = input
        .codex_evidence_record
        .ok_or(OrdinaryAuthorityBootstrapError::MissingCodexEvidence)?;
    require_keys(
        &input.codex_trusted_keys,
        OrdinaryAuthorityBootstrapError::MissingCodexKeys,
    )?;
    let claude_evidence_record = input
        .claude_evidence_record
        .ok_or(OrdinaryAuthorityBootstrapError::MissingClaudeEvidence)?;
    require_keys(
        &input.claude_trusted_keys,
        OrdinaryAuthorityBootstrapError::MissingClaudeKeys,
    )?;
    let compatibility_cache = input
        .compatibility_cache
        .ok_or(OrdinaryAuthorityBootstrapError::MissingCompatibilityCache)?;
    require_keys(
        &input.compatibility_keys,
        OrdinaryAuthorityBootstrapError::MissingCompatibilityKeys,
    )?;
    Ok(Some(OrdinaryAuthorityBootstrapConfig {
        codex_evidence_record,
        codex_trusted_keys: input.codex_trusted_keys,
        claude_evidence_record,
        claude_trusted_keys: input.claude_trusted_keys,
        compatibility_cache,
        compatibility_keys: input.compatibility_keys,
    }))
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
