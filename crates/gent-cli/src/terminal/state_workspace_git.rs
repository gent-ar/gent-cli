use super::UiState;
use gent_types::WorkspaceGitReport;

impl UiState {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_workspace_git(
        mut self,
        workspace_id: &str,
        report: Option<WorkspaceGitReport>,
    ) -> Self {
        self.set_workspace_git(workspace_id.into(), report);
        self
    }

    pub(crate) fn set_workspace_git(
        &mut self,
        workspace_id: String,
        report: Option<WorkspaceGitReport>,
    ) {
        self.workspace_git.insert(workspace_id, report);
    }

    #[must_use]
    pub(crate) fn workspace_git_due(&self, turn_ended: bool) -> Option<String> {
        self.selected_workspace_id()
            .filter(|workspace_id| turn_ended || !self.workspace_git.contains_key(*workspace_id))
            .map(str::to_owned)
    }

    fn selected_workspace_git(&self) -> Option<&WorkspaceGitReport> {
        self.selected_workspace_id()
            .and_then(|workspace_id| self.workspace_git.get(workspace_id))
            .and_then(Option::as_ref)
    }

    #[must_use]
    pub(crate) fn selected_changed_file_count(&self) -> Option<u32> {
        self.selected_workspace_git()
            .map(|report| u32::try_from(report.files.len()).unwrap_or(u32::MAX))
    }

    #[must_use]
    pub(crate) fn selected_git_branch(&self) -> Option<&str> {
        self.selected_workspace_git()
            .and_then(|report| report.branch.as_deref())
    }
}

#[cfg(test)]
#[path = "state_workspace_git_tests.rs"]
pub(crate) mod tests;
