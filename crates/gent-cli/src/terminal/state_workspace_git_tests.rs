use std::collections::BTreeMap;

use gent_types::{
    ConversationListItem, WorkspaceGitFileStatus, WorkspaceGitRemoteStatus, WorkspaceGitReport,
};

use crate::terminal::{ConversationMetadata, UiState};

pub(crate) fn report(branch: &str, changed_files: usize) -> WorkspaceGitReport {
    WorkspaceGitReport {
        repository_root: "/workspace".into(),
        branch: Some(branch.into()),
        files: (0..changed_files)
            .map(|index| WorkspaceGitFileStatus {
                index_status: ' ',
                worktree_status: 'M',
                path: format!("file-{index}.rs"),
                original_path: None,
            })
            .collect(),
        worktrees: Vec::new(),
        recent_commits: Vec::new(),
        branches: Vec::new(),
        stashes: Vec::new(),
        remote_status: WorkspaceGitRemoteStatus {
            ahead: 0,
            behind: 0,
            tracking_branch: None,
        },
    }
}

pub(crate) fn metadata(conversation_id: &str) -> BTreeMap<String, ConversationMetadata> {
    BTreeMap::from([(
        conversation_id.into(),
        ConversationMetadata {
            workspace_id: Some("workspace".into()),
            workspace_path: Some("/workspace".into()),
            ..ConversationMetadata::default()
        },
    )])
}

#[test]
fn workspace_git_is_due_once_per_workspace_and_again_only_when_a_turn_ends() {
    let mut state = UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_metadata(metadata("one"));
    assert_eq!(state.workspace_git_due(false).as_deref(), Some("workspace"));
    state.set_workspace_git("workspace".into(), Some(report("main", 2)));
    assert_eq!(state.workspace_git_due(false), None);
    assert_eq!(state.workspace_git_due(true).as_deref(), Some("workspace"));
    assert_eq!(state.selected_git_branch(), Some("main"));
    assert_eq!(state.selected_changed_file_count(), Some(2));
    state.set_workspace_git("workspace".into(), None);
    assert_eq!(state.workspace_git_due(false), None);
    assert_eq!(state.selected_git_branch(), None);
    assert_eq!(state.selected_changed_file_count(), None);
}
