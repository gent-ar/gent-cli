use std::path::PathBuf;

use gent_types::{AgentChatEffort, AgentChatMode, AgentChatSelection, PermissionMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelIdentity {
    pub(crate) model_id: String,
    effort: AgentChatEffort,
    mode: AgentChatMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionIdentity {
    workspace: PathBuf,
    permission_mode: PermissionMode,
    mcp_config_digest: Option<String>,
    tool_source_ids: Vec<String>,
    conversation_id: String,
    authority: Option<gent_types::SandboxPermissionBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeIdentity {
    pub(crate) model: ModelIdentity,
    pub(crate) session: SessionIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeReuse {
    Ready,
    RebindSession,
    Relaunch,
}

impl RuntimeIdentity {
    pub(crate) fn new(
        selection: &AgentChatSelection,
        conversation_id: String,
        workspace: PathBuf,
        permission_mode: PermissionMode,
        mcp_config_digest: Option<String>,
        tool_source_ids: &[String],
    ) -> Self {
        let mut tool_source_ids = tool_source_ids.to_vec();
        tool_source_ids.sort();
        Self {
            model: ModelIdentity {
                model_id: selection.model.clone(),
                effort: selection.effort,
                mode: selection.mode,
            },
            session: SessionIdentity {
                workspace,
                permission_mode,
                mcp_config_digest,
                tool_source_ids,
                conversation_id,
                authority: None,
            },
        }
    }

    pub(crate) fn permission_binding(&self) -> Option<&gent_types::SandboxPermissionBinding> {
        self.session.authority.as_ref()
    }

    pub(crate) fn with_permission_binding(
        mut self,
        binding: gent_types::SandboxPermissionBinding,
    ) -> Self {
        self.session.authority = Some(binding);
        self
    }

    #[must_use]
    pub(crate) fn reuse_from(&self, running: &Self) -> RuntimeReuse {
        if running == self {
            return RuntimeReuse::Ready;
        }
        if self.session.authority.is_some() || running.session.authority.is_some() {
            return RuntimeReuse::Relaunch;
        }
        if running.model == self.model {
            return RuntimeReuse::RebindSession;
        }
        RuntimeReuse::Relaunch
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeIdentity, RuntimeReuse};
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, PermissionMode,
    };
    use std::path::PathBuf;

    fn selection() -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen3-1-7b-q4-k-m".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        }
    }

    fn identity(
        selection: &AgentChatSelection,
        conversation_id: &str,
        workspace: &str,
        permission_mode: PermissionMode,
    ) -> RuntimeIdentity {
        RuntimeIdentity::new(
            selection,
            conversation_id.into(),
            PathBuf::from(workspace),
            permission_mode,
            Some("mcp".into()),
            &[],
        )
    }

    fn running() -> RuntimeIdentity {
        identity(
            &selection(),
            "conversation-1",
            "/workspace",
            PermissionMode::AskEveryTime,
        )
    }

    #[test]
    fn contained_identity_changes_never_rebind_an_existing_process() {
        use gent_types::{HostEpoch, PolicyRecord, PolicyScope, SandboxPermissionBinding};
        let selection = selection();
        let policy = PolicyRecord {
            policy_id: "p".into(),
            workspace_id: "w".into(),
            revision: 1,
            scope: PolicyScope::ProviderPermissions,
            mode: PermissionMode::Bypass,
            containment: gent_types::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        };
        let first = identity(
            &selection,
            "conversation",
            "/workspace",
            PermissionMode::Bypass,
        )
        .with_permission_binding(
            SandboxPermissionBinding::new(
                "conversation".into(),
                "one".into(),
                policy.clone(),
                HostEpoch(1),
            )
            .unwrap(),
        );
        assert_eq!(first.reuse_from(&first), RuntimeReuse::Ready);
        let next = identity(
            &selection,
            "conversation",
            "/workspace",
            PermissionMode::Bypass,
        )
        .with_permission_binding(
            SandboxPermissionBinding::new(
                "conversation".into(),
                "two".into(),
                policy,
                HostEpoch(1),
            )
            .unwrap(),
        );
        assert_eq!(next.reuse_from(&first), RuntimeReuse::Relaunch);
    }

    #[test]
    fn an_unchanged_selection_reuses_the_running_runtime() {
        assert_eq!(running().reuse_from(&running()), RuntimeReuse::Ready);
    }

    #[test]
    fn only_the_session_changes_when_the_model_effort_and_mode_hold() {
        for changed in [
            identity(
                &selection(),
                "conversation-2",
                "/workspace",
                PermissionMode::AskEveryTime,
            ),
            identity(
                &selection(),
                "conversation-1",
                "/other",
                PermissionMode::AskEveryTime,
            ),
            identity(
                &selection(),
                "conversation-1",
                "/workspace",
                PermissionMode::Bypass,
            ),
            RuntimeIdentity::new(
                &selection(),
                "conversation-1".into(),
                PathBuf::from("/workspace"),
                PermissionMode::AskEveryTime,
                Some("other-mcp".into()),
                &[],
            ),
            RuntimeIdentity::new(
                &selection(),
                "conversation-1".into(),
                PathBuf::from("/workspace"),
                PermissionMode::AskEveryTime,
                Some("mcp".into()),
                &["source".into()],
            ),
        ] {
            assert_eq!(
                changed.reuse_from(&running()),
                RuntimeReuse::RebindSession,
                "{changed:?} must keep the loaded model"
            );
        }
    }

    #[test]
    fn a_model_effort_or_mode_change_relaunches_the_whole_runtime() {
        let mut other_model = selection();
        other_model.model = "qwen3-4b".into();
        let mut other_effort = selection();
        other_effort.effort = AgentChatEffort::High;
        let mut other_mode = selection();
        other_mode.mode = AgentChatMode::Ask;
        for changed in [other_model, other_effort, other_mode] {
            assert_eq!(
                identity(
                    &changed,
                    "conversation-1",
                    "/workspace",
                    PermissionMode::AskEveryTime
                )
                .reuse_from(&running()),
                RuntimeReuse::Relaunch
            );
        }
    }

    #[test]
    fn tool_source_order_never_changes_the_identity() {
        let ordered = RuntimeIdentity::new(
            &selection(),
            "conversation-1".into(),
            PathBuf::from("/workspace"),
            PermissionMode::AskEveryTime,
            Some("mcp".into()),
            &["a".into(), "b".into()],
        );
        let reversed = RuntimeIdentity::new(
            &selection(),
            "conversation-1".into(),
            PathBuf::from("/workspace"),
            PermissionMode::AskEveryTime,
            Some("mcp".into()),
            &["b".into(), "a".into()],
        );
        assert_eq!(ordered.reuse_from(&reversed), RuntimeReuse::Ready);
    }
}
