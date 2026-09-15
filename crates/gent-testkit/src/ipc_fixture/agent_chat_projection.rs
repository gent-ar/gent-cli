use super::{FixtureFrame, frames::canonical};
use gent_protocol::{AgentChatProjectionFrame, WireFrame};
use gent_types::{AgentChatCommandRejection, AgentChatRejection};

pub(super) fn validate_chat_projection(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<AgentChatProjectionFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            AgentChatProjectionFrame::ConversationSnapshotRequest { .. },
            AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. }
        ] if snapshot.transcript_truncated
    )
    .then_some(())
    .ok_or_else(|| "must contain a snapshot request and an explicitly truncated snapshot".into())
}

pub(super) fn validate_chat_rejections(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<WireFrame> = canonical(records)?;
    let codes = frames
        .iter()
        .map(|frame| match frame {
            WireFrame::Error { code, .. } => Some(code.as_str()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("must contain only typed error frames")?;
    let expected = [
        AgentChatRejection::SelectionSwitchBlockedByActiveTurn,
        AgentChatRejection::SelectionSwitchParentNotCurrent,
        AgentChatRejection::SelectionSwitchBlockedByProvisioning,
        AgentChatRejection::QueuedPromptNotCancelable,
        AgentChatRejection::QueuedPromptNotSteerable,
        AgentChatRejection::LifecycleRecoveryInProgress,
        AgentChatRejection::LifecycleShuttingDown,
        AgentChatRejection::ConversationNotFound,
        AgentChatRejection::SelectionModelUnavailable,
        AgentChatRejection::SelectionEffortUnavailable,
    ]
    .map(AgentChatRejection::code);
    let name = || String::from("context");
    let commands = [
        AgentChatCommandRejection::UnknownCommand { name: name() },
        AgentChatCommandRejection::UnsupportedCommand {
            name: name(),
            reason: name(),
            use_instead: None,
        },
        AgentChatCommandRejection::ClientActionCommand { name: name() },
        AgentChatCommandRejection::CommandRequiresConversation { name: name() },
        AgentChatCommandRejection::CommandBlockedByActiveTurn { name: name() },
        AgentChatCommandRejection::CommandArgumentsInvalid {
            name: name(),
            hint: name(),
        },
        AgentChatCommandRejection::CommandRequiresProviderSession { name: name() },
        AgentChatCommandRejection::CommandCatalogLoading { name: name() },
        AgentChatCommandRejection::SlashCommandRequiresInvoke { name: name() },
    ];
    let expected: Vec<&str> = expected
        .into_iter()
        .chain(commands.iter().map(AgentChatCommandRejection::code))
        .collect();
    (codes == expected)
        .then_some(())
        .ok_or_else(|| "must list every typed agent-chat rejection code in order".into())
}
