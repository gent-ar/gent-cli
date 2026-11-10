use std::{collections::BTreeMap, path::PathBuf};

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
};
use gent_types::ConversationListItem;

use crate::{chat_cli, conversation_index, local_ipc, terminal};

use super::{metadata::initial_metadata, provider::model_catalog, read_view};

pub(super) struct Startup {
    pub(super) state: terminal::UiState,
    pub(super) capabilities: Vec<String>,
}

pub(super) async fn load(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    browse_only: bool,
) -> Result<Startup, Box<dyn std::error::Error>> {
    let mut index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    let (_, capabilities) =
        local_ipc::connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    let enabled = !browse_only
        && capabilities
            .0
            .iter()
            .any(|value| value == AGENT_CHAT_INTENTS_CAPABILITY);
    let mut metadata = initial_metadata(
        &index,
        &capabilities.0,
        data_dir.clone(),
        no_autostart,
        false,
    )
    .await;
    by_recency(&mut index, &metadata);
    let workspace = std::env::current_dir()?;
    let mut selected = if browse_only {
        index.first().map(|item| item.conversation_id.clone())
    } else {
        latest_in_workspace(&index, &metadata, &workspace)
    };
    if enabled && selected.is_none() {
        let (conversation_id, _) =
            chat_cli::create(data_dir.clone(), no_autostart, None, Some(workspace)).await?;
        index = conversation_index::request(data_dir.clone(), no_autostart).await?;
        metadata = initial_metadata(
            &index,
            &capabilities.0,
            data_dir.clone(),
            no_autostart,
            false,
        )
        .await;
        by_recency(&mut index, &metadata);
        selected = Some(conversation_id.0);
    }
    let view = initial_view(
        selected.clone(),
        &capabilities.0,
        data_dir.clone(),
        no_autostart,
    )
    .await;
    let model_catalog = model_catalog(data_dir.clone(), no_autostart, &capabilities.0).await;
    let command_catalog = crate::command_catalog_cli::read(data_dir.clone(), no_autostart, None)
        .await
        .ok()
        .flatten();
    let mut sessions = Vec::new();
    let workspaces = metadata
        .values()
        .filter(|_| browse_only)
        .filter_map(|item| item.workspace_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for workspace_id in workspaces {
        if let Ok(items) =
            crate::session_cli::list(data_dir.clone(), no_autostart, workspace_id).await
        {
            sessions.extend(items);
        }
    }
    Ok(Startup {
        state: terminal::UiState::new(index)
            .with_chat_input(enabled)
            .with_metadata(metadata)
            .with_selected_conversation(selected.as_deref())
            .with_sessions(sessions)
            .with_model_catalog(model_catalog)
            .with_view(view)
            .with_command_catalog(command_catalog),
        capabilities: capabilities.0,
    })
}

pub(super) async fn initial_view(
    conversation_id: Option<String>,
    capabilities: &[String],
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Option<terminal::ConversationView> {
    let conversation_id = conversation_id?;
    capabilities
        .iter()
        .any(|capability| {
            capability == CONVERSATION_STATUS_CAPABILITY
                || capability == AGENT_CHAT_TRANSCRIPT_CAPABILITY
        })
        .then(|| {
            Box::pin(read_view(
                data_dir,
                no_autostart,
                conversation_id,
                capabilities,
            ))
        })?
        .await
        .ok()
}

pub(super) fn by_recency(
    index: &mut [ConversationListItem],
    metadata: &BTreeMap<String, terminal::ConversationMetadata>,
) {
    index.sort_by_key(|item| {
        std::cmp::Reverse(
            metadata
                .get(&item.conversation_id)
                .map_or(0, |value| value.updated_at_unix_ms),
        )
    });
}

pub(super) fn latest_in_workspace(
    index: &[ConversationListItem],
    metadata: &BTreeMap<String, terminal::ConversationMetadata>,
    workspace: &std::path::Path,
) -> Option<String> {
    let workspace = normalized(&workspace.to_string_lossy());
    index
        .iter()
        .find(|item| {
            metadata
                .get(&item.conversation_id)
                .and_then(|value| value.workspace_path.as_deref())
                .is_some_and(|path| normalized(path) == workspace)
        })
        .map(|item| item.conversation_id.clone())
}

fn normalized(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path.trim_end_matches(['\\', '/']);
    if cfg!(windows) {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gent_types::ConversationListItem;

    use super::{by_recency, latest_in_workspace};
    use crate::terminal::ConversationMetadata;

    fn item(id: &str) -> ConversationListItem {
        ConversationListItem {
            conversation_id: id.into(),
            run_count: 1,
        }
    }

    #[test]
    fn startup_opens_the_newest_conversation_of_the_current_workspace() {
        let mut metadata = BTreeMap::new();
        for (id, path, updated) in [
            ("elsewhere", r"\\?\X:\Dev\video-studio", 300),
            ("old-here", r"\\?\X:\work\ws1", 100),
            ("new-here", r"\\?\X:\work\ws1\", 200),
        ] {
            metadata.insert(
                id.to_owned(),
                ConversationMetadata {
                    workspace_path: Some(path.into()),
                    updated_at_unix_ms: updated,
                    ..ConversationMetadata::default()
                },
            );
        }
        let mut index = vec![item("old-here"), item("elsewhere"), item("new-here")];
        by_recency(&mut index, &metadata);
        assert_eq!(index[0].conversation_id, "elsewhere");
        let workspace = if cfg!(windows) {
            r"x:\work\WS1"
        } else {
            r"X:\work\ws1"
        };
        assert_eq!(
            latest_in_workspace(&index, &metadata, std::path::Path::new(workspace)).as_deref(),
            Some("new-here")
        );
        assert_eq!(
            latest_in_workspace(&index, &metadata, std::path::Path::new("/nowhere")),
            None
        );
    }
}
