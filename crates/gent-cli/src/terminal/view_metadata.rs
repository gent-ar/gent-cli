#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConversationMetadata {
    pub(crate) permission_mode: gent_types::PermissionMode,
    pub(crate) title: Option<String>,
    pub(crate) recap: Option<String>,
    pub(crate) preview: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
    pub(crate) mcp_server_count: u16,
    pub(crate) mcp_server_names: Vec<String>,
    pub(crate) automation_count: u16,
    pub(crate) automation_names: Vec<String>,
    pub(crate) automations: Vec<gent_types::AutomationDefinition>,
    pub(crate) automation_runs: Vec<gent_types::AutomationRunSummary>,
    pub(crate) forge_count: u16,
    pub(crate) forge_names: Vec<String>,
    pub(crate) changed_file_count: Option<u32>,
    pub(crate) git_branch: Option<String>,
}
