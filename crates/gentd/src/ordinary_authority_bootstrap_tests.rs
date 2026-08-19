use std::path::PathBuf;

use super::{
    OrdinaryAuthorityBootstrapError, OrdinaryAuthorityBootstrapInput,
    OrdinaryProviderBootstrapConfig, parse,
};

const KEY: &str = "review:key";

fn complete_input() -> OrdinaryAuthorityBootstrapInput {
    OrdinaryAuthorityBootstrapInput {
        enabled: true,
        agent_chat_authority: false,
        codex_evidence_record: Some(PathBuf::from("/unread/codex-evidence.json")),
        codex_trusted_keys: vec![KEY.into()],
        claude_evidence_record: Some(PathBuf::from("/unread/claude-evidence.json")),
        claude_trusted_keys: vec![KEY.into()],
        compatibility_cache: Some(PathBuf::from("/unread/compatibility.json")),
        compatibility_keys: vec![KEY.into()],
    }
}

#[test]
fn default_observer_is_unavailable_without_reading_authority_records() {
    assert_eq!(
        parse(OrdinaryAuthorityBootstrapInput::default()).unwrap(),
        None
    );
}

#[test]
fn observer_rejects_even_one_authority_setting_without_reading_its_path() {
    let input = OrdinaryAuthorityBootstrapInput {
        codex_evidence_record: Some(PathBuf::from("/does-not-exist/evidence.json")),
        ..OrdinaryAuthorityBootstrapInput::default()
    };
    assert_eq!(
        parse(input),
        Err(OrdinaryAuthorityBootstrapError::SettingsRequireOptIn)
    );
}

#[test]
fn authority_rejects_the_chat_only_profile_before_examining_evidence() {
    let mut input = complete_input();
    input.agent_chat_authority = true;
    assert_eq!(
        parse(input),
        Err(OrdinaryAuthorityBootstrapError::ConflictsWithAgentChatAuthority)
    );
}

#[test]
fn authority_requires_one_complete_selected_provider() {
    let mut input = OrdinaryAuthorityBootstrapInput {
        enabled: true,
        ..OrdinaryAuthorityBootstrapInput::default()
    };
    assert_eq!(
        parse(input.clone()),
        Err(OrdinaryAuthorityBootstrapError::MissingProvider)
    );
    input.codex_evidence_record = Some(PathBuf::from("/unread/codex.json"));
    assert_eq!(
        parse(input.clone()),
        Err(OrdinaryAuthorityBootstrapError::MissingCodexKeys)
    );
    input.codex_evidence_record = None;
    input.codex_trusted_keys.push(KEY.into());
    assert_eq!(
        parse(input),
        Err(OrdinaryAuthorityBootstrapError::MissingCodexEvidence)
    );
}

#[test]
fn one_provider_keeps_an_unavailable_other_provider_out_of_preflight() {
    let mut input = OrdinaryAuthorityBootstrapInput {
        enabled: true,
        ..OrdinaryAuthorityBootstrapInput::default()
    };
    input.codex_evidence_record = Some(PathBuf::from("/unread/codex.json"));
    input.codex_trusted_keys.push(KEY.into());
    input.compatibility_cache = Some(PathBuf::from("/unread/compatibility.json"));
    input.compatibility_keys.push(KEY.into());
    let config = parse(input).unwrap().unwrap();
    assert_eq!(
        config.providers,
        vec![OrdinaryProviderBootstrapConfig::Codex {
            evidence_record: PathBuf::from("/unread/codex.json"),
            trusted_keys: vec![KEY.into()],
        }]
    );
}

#[test]
fn authority_requires_complete_signed_compatibility_inputs() {
    let mut input = complete_input();
    input.compatibility_cache = None;
    assert_eq!(
        parse(input.clone()),
        Err(OrdinaryAuthorityBootstrapError::MissingCompatibilityCache)
    );
    input.compatibility_cache = Some(PathBuf::from("/unread/compatibility.json"));
    input.compatibility_keys.clear();
    assert_eq!(
        parse(input),
        Err(OrdinaryAuthorityBootstrapError::MissingCompatibilityKeys)
    );
}

#[test]
fn complete_authority_input_preserves_only_later_preflight_material() {
    let config = parse(complete_input()).unwrap().unwrap();
    assert_eq!(config.providers.len(), 2);
    assert_eq!(
        config.compatibility_cache,
        PathBuf::from("/unread/compatibility.json")
    );
    assert!(matches!(
        &config.providers[0],
        OrdinaryProviderBootstrapConfig::Codex { evidence_record, trusted_keys }
            if evidence_record == &PathBuf::from("/unread/codex-evidence.json")
                && trusted_keys == &vec![KEY]
    ));
    assert!(matches!(
        &config.providers[1],
        OrdinaryProviderBootstrapConfig::Claude { evidence_record, trusted_keys }
            if evidence_record == &PathBuf::from("/unread/claude-evidence.json")
                && trusted_keys == &vec![KEY]
    ));
    assert_eq!(config.compatibility_keys, vec![KEY]);
}
