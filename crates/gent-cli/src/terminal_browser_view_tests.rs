use gent_protocol::agent_chat_commands::{CommandCatalog, CommandCatalogScope, CommandListing};
use gent_types::AgentChatProvider;

use super::{CommandScope, after_login, remember, remembered_for, settled_for};

fn catalog(listing: CommandListing) -> CommandCatalog {
    CommandCatalog {
        scope: CommandCatalogScope {
            conversation_id: None,
            workspace_path: None,
            provider: Some(AgentChatProvider::Claude),
        },
        revision: "sha256:0".into(),
        provider_version: None,
        listing,
        commands: Vec::new(),
    }
}

fn scope(conversation: &str, provider: AgentChatProvider) -> CommandScope {
    (conversation.into(), Some(provider), Some("/work".into()))
}

#[test]
fn a_settled_catalog_is_reused_until_the_conversation_scope_changes() {
    let claude = scope("c-1", AgentChatProvider::Claude);
    let ready = Some((claude.clone(), catalog(CommandListing::Ready)));
    assert!(settled_for(ready.clone(), &claude).is_some());
    let failed = CommandListing::Failed {
        message: "claude is not signed in".into(),
    };
    assert!(settled_for(Some((claude.clone(), catalog(failed))), &claude).is_some());
    assert!(settled_for(ready.clone(), &scope("c-2", AgentChatProvider::Claude)).is_none());
    assert!(settled_for(ready, &scope("c-1", AgentChatProvider::Codex)).is_none());
}

#[test]
fn a_loading_catalog_is_read_again() {
    let claude = scope("c-1", AgentChatProvider::Claude);
    let loading = Some((claude.clone(), catalog(CommandListing::Loading)));
    assert!(settled_for(loading, &claude).is_none());
    assert!(settled_for(None, &claude).is_none());
}

#[test]
fn only_a_successful_login_makes_the_next_view_read_the_catalog_again() {
    let claude = scope("c-login", AgentChatProvider::Claude);
    let signed_out = CommandListing::Failed {
        message: "claude is not signed in".into(),
    };
    remember(claude.clone(), catalog(signed_out));
    assert!(remembered_for(&claude).is_some());
    assert_eq!(after_login::<(), ()>(Err(())), Err(()));
    assert!(remembered_for(&claude).is_some());
    assert_eq!(after_login::<_, ()>(Ok("signed in")), Ok("signed in"));
    assert!(remembered_for(&claude).is_none());
    remember(claude.clone(), catalog(CommandListing::Ready));
    assert!(remembered_for(&claude).is_some());
}
