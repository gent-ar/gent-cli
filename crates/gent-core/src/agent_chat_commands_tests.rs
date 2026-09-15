use gent_types::{
    AgentChatClientAction, AgentChatCommandAvailability, AgentChatCommandDescriptor,
    AgentChatCommandDispatch, AgentChatCommandIntent, AgentChatCommandOrigin, AgentChatProvider,
};

use super::{command_catalog, reserved_names, resolve_command};

fn claude(name: &str, aliases: &[&str]) -> AgentChatCommandDescriptor {
    AgentChatCommandDescriptor {
        name: name.into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        description: format!("Claude {name}"),
        argument_hint: None,
        origin: AgentChatCommandOrigin::ProviderBuiltin {
            provider: AgentChatProvider::Claude,
        },
        dispatch: AgentChatCommandDispatch::ProviderNative,
        availability: AgentChatCommandAvailability {
            requires_conversation: true,
            blocked_while_turn_active: true,
        },
    }
}

#[test]
fn reserved_gent_names_win_over_provider_names_and_aliases() {
    let catalog = command_catalog(
        Some(AgentChatProvider::Claude),
        [
            claude("clear", &["new", "reset"]),
            claude("model", &[]),
            claude("code-review", &["review", "fork"]),
        ],
    );
    let clear = resolve_command(&catalog, "reset").unwrap();
    assert_eq!(clear.origin, AgentChatCommandOrigin::Gent);
    assert_eq!(
        clear.dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::ClearContext
        }
    );
    assert_eq!(
        resolve_command(&catalog, "model").unwrap().origin,
        AgentChatCommandOrigin::Gent
    );
    let review = resolve_command(&catalog, "review").unwrap();
    assert_eq!(review.name, "code-review");
    assert_eq!(review.aliases, ["review"]);
    assert_eq!(
        resolve_command(&catalog, "fork").unwrap().origin,
        AgentChatCommandOrigin::Gent
    );
}

#[test]
fn double_underscore_commands_are_never_exposed_and_duplicates_keep_the_first() {
    let catalog = command_catalog(
        Some(AgentChatProvider::Claude),
        [
            claude("__remote-workflow", &[]),
            claude("usage", &[]),
            claude("usage", &["cost"]),
        ],
    );
    assert!(resolve_command(&catalog, "__remote-workflow").is_none());
    assert_eq!(
        catalog
            .iter()
            .filter(|command| command.name == "usage")
            .count(),
        1
    );
    assert!(resolve_command(&catalog, "cost").is_none());
}

#[test]
fn every_gent_command_is_valid_unique_and_reserved() {
    let catalog = command_catalog(None, []);
    let mut seen = Vec::new();
    for command in &catalog {
        assert!(command.is_valid(), "{command:?}");
        for name in std::iter::once(&command.name).chain(&command.aliases) {
            assert!(!seen.contains(name), "duplicate command name {name}");
            assert!(reserved_names(None).contains(&name.as_str()));
            seen.push(name.clone());
        }
    }
    assert_eq!(
        resolve_command(&catalog, "resume").unwrap().dispatch,
        AgentChatCommandDispatch::ClientAction {
            action: AgentChatClientAction::ConversationPicker
        }
    );
    assert_eq!(
        resolve_command(&catalog, "compact").unwrap().dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::Compact
        }
    );
}

#[test]
fn compact_is_claude_native_and_a_gent_intent_for_codex_and_local_models() {
    let claude_catalog = command_catalog(Some(AgentChatProvider::Claude), [claude("compact", &[])]);
    let compact = resolve_command(&claude_catalog, "compact").unwrap();
    assert_eq!(
        compact.origin,
        AgentChatCommandOrigin::ProviderBuiltin {
            provider: AgentChatProvider::Claude
        }
    );
    assert!(!reserved_names(Some(AgentChatProvider::Claude)).contains(&"compact"));
    let codex = command_catalog(Some(AgentChatProvider::Codex), []);
    assert_eq!(
        resolve_command(&codex, "compact").unwrap().dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::Compact
        }
    );
    let local = command_catalog(Some(AgentChatProvider::Claurst), []);
    let local_compact = resolve_command(&local, "compact").unwrap();
    assert_eq!(
        local_compact.dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::Compact
        }
    );
    assert_eq!(local_compact.origin, AgentChatCommandOrigin::Gent);
    assert!(local_compact.availability.blocked_while_turn_active);
    assert!(local_compact.availability.requires_conversation);
    assert!(reserved_names(Some(AgentChatProvider::Claurst)).contains(&"compact"));
}
