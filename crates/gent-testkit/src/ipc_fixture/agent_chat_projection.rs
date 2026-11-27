use super::{FixtureFrame, frames::canonical};
use gent_protocol::{AgentChatProjectionFrame, WireFrame};
use gent_types::{AgentChatCommandRejection, AgentChatRejection};

pub(super) fn validate_chat_projection(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<AgentChatProjectionFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            AgentChatProjectionFrame::ConversationSnapshotRequest { .. },
            AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. },
            AgentChatProjectionFrame::FollowConversation { .. },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Summary { summary, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::UnreadableRecord { .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: started, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: queued, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: withdrawn, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: stopping, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: cleared, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: compacted, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: removing, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: changed, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Activity { fact: skipped, .. },
                ..
            },
            AgentChatProjectionFrame::Delta {
                delta: gent_protocol::AgentChatProjectionDelta::Catalog { catalog, .. },
                ..
            }
        ] if snapshot.transcript_truncated
            && !snapshot.unreadable.is_empty()
            && summary.queue_delivery.is_some()
            && matches!(
                **started,
                gent_types::ConversationActivityFact::TurnStarted { .. }
            )
            && matches!(
                **queued,
                gent_types::ConversationActivityFact::PromptQueued {
                    queue_delivery: Some(_),
                    cancelable: Some(_),
                    ..
                }
            )
            && matches!(
                **withdrawn,
                gent_types::ConversationActivityFact::PromptWithdrawn { .. }
            )
            && matches!(
                **stopping,
                gent_types::ConversationActivityFact::InterruptRequested { .. }
            )
            && matches!(
                **cleared,
                gent_types::ConversationActivityFact::ContextCleared {
                    command: Some(gent_types::ContextClearCommand::Clear),
                    ..
                }
            )
            && matches!(
                **compacted,
                gent_types::ConversationActivityFact::ContextCompacted { .. }
            )
            && matches!(
                **removing,
                gent_types::ConversationActivityFact::PromptCancelRequested { .. }
            )
            && matches!(
                &**changed,
                gent_types::ConversationActivityFact::WorkspaceChanges { files, truncated: false, .. }
                    if !files.is_empty()
            )
            && matches!(
                **skipped,
                gent_types::ConversationActivityFact::DecisionSettled {
                    outcome: Some(gent_types::DecisionOutcome::Skipped),
                    ..
                }
            )
            && catalog.catalog_id == gent_protocol::COMMAND_CATALOG_ID
            && catalog.records.is_empty()
    )
    .then_some(())
    .ok_or_else(|| {
        "must contain a truncated snapshot naming an unreadable record, a summary delta with queueDelivery, an unreadable record delta, a turnStarted, a promptQueued with queueDelivery and cancelable, a promptWithdrawn, an interruptRequested, a contextCleared naming its clear command, a contextCompacted, a promptCancelRequested, a workspaceChanges with files, a decisionSettled with its outcome and a commands catalog delta".into()
    })
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
        AgentChatRejection::PlanApprovalBlockedByActiveTurn,
        AgentChatRejection::PlanNotReviewable,
        AgentChatRejection::PlanApprovalParentNotCurrent,
        AgentChatRejection::PlanApprovalPolicyChanged,
        AgentChatRejection::SelectionSwitchParentNotCurrent,
        AgentChatRejection::SelectionSwitchBlockedByProvisioning,
        AgentChatRejection::QueuedPromptNotCancelable,
        AgentChatRejection::QueuedPromptNotSteerable,
        AgentChatRejection::LifecycleRecoveryInProgress,
        AgentChatRejection::LifecycleShuttingDown,
        AgentChatRejection::ConversationNotFound,
        AgentChatRejection::SelectionModelUnavailable,
        AgentChatRejection::SelectionEffortUnavailable,
        AgentChatRejection::ConversationArchived,
        AgentChatRejection::NothingToInterrupt,
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
