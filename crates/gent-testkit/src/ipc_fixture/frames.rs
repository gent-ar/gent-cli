use super::{FixtureFrame, wire::encoded_frame_hex};
use gent_protocol::{
    AgentChatConversationFrame, AgentChatIntentFrame, AgentChatTranscriptFrame, EventStreamFrame,
    PermissionPolicyFrame, WireFrame, negotiate,
};

pub(super) fn validate_handshake(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<WireFrame> = canonical(records)?;
    let [WireFrame::Hello(hello), WireFrame::Negotiated(negotiated)] = frames.as_slice() else {
        return Err("must contain hello then negotiated".into());
    };
    let expected =
        negotiate(hello, 1, 1, &hello.capabilities).map_err(|error| error.to_string())?;
    (negotiated == &expected)
        .then_some(())
        .ok_or_else(|| "negotiated frame does not match hello intersection".into())
}

pub(super) fn validate_core(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<WireFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            WireFrame::Command(_),
            WireFrame::Receipt(_),
            WireFrame::Subscribe { .. },
            WireFrame::Events { .. },
            WireFrame::Error { .. }
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain command, receipt, subscribe, events, error".into())
}

pub(super) fn validate_event_stream(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<EventStreamFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            EventStreamFrame::Attach { .. },
            EventStreamFrame::Replay { .. },
            EventStreamFrame::Events { .. },
            EventStreamFrame::Ack { .. },
            EventStreamFrame::Error { .. }
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain attach, replay, events, ack, error".into())
}

pub(super) fn validate_chat_conversations(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<AgentChatConversationFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            AgentChatConversationFrame::SummaryRequest { .. },
            AgentChatConversationFrame::Summary(_),
            AgentChatConversationFrame::DetailRequest { .. },
            AgentChatConversationFrame::Detail(_)
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain conversation read request and response frames".into())
}

pub(super) fn validate_chat_transcript(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<AgentChatTranscriptFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            AgentChatTranscriptFrame::PageRequest { .. },
            AgentChatTranscriptFrame::Page(_)
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain transcript page request and response frames".into())
}

pub(super) fn validate_chat_intents(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<AgentChatIntentFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            AgentChatIntentFrame::CreateConversation { .. },
            AgentChatIntentFrame::SendPrompt { .. },
            AgentChatIntentFrame::QueuePrompt { .. },
            AgentChatIntentFrame::Interrupt { .. },
            AgentChatIntentFrame::SteerQueuedPrompt { .. },
            AgentChatIntentFrame::QueuedPromptSteered { .. },
            AgentChatIntentFrame::ContinueFromSavedHistory { .. },
            AgentChatIntentFrame::Decision { .. },
            AgentChatIntentFrame::Subscribe { .. },
            AgentChatIntentFrame::SubscriptionEvent { .. },
            AgentChatIntentFrame::SubscriptionEnded { .. },
            AgentChatIntentFrame::Accepted { .. }
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain every reserved agent-chat intent and subscription frame".into())
}

pub(super) fn validate_permission_policy(records: &[FixtureFrame]) -> Result<(), String> {
    let frames: Vec<PermissionPolicyFrame> = canonical(records)?;
    matches!(
        frames.as_slice(),
        [
            PermissionPolicyFrame::Current { .. },
            PermissionPolicyFrame::CurrentPolicy { .. },
            PermissionPolicyFrame::Save { .. },
            PermissionPolicyFrame::Saved { .. }
        ]
    )
    .then_some(())
    .ok_or_else(|| "must contain current, current policy, save, and saved frames".into())
}

pub(super) fn validate_model_catalog(records: &[FixtureFrame]) -> Result<(), String> {
    use gent_protocol::model_catalog::ModelCatalogFrame;
    let frames: Vec<ModelCatalogFrame> = canonical(records)?;
    let valid = frames.iter().all(|frame| frame.validate().is_ok());
    (valid
        && matches!(
            frames.as_slice(),
            [
                ModelCatalogFrame::ReadModelCatalog { .. },
                ModelCatalogFrame::ModelCatalog { .. },
                ModelCatalogFrame::SetDefaultModel { .. },
                ModelCatalogFrame::StartModelDownload { .. },
                ModelCatalogFrame::CancelModelDownload { .. }
            ]
        ))
    .then_some(())
    .ok_or_else(|| "must contain valid read, catalog, default, download, and cancel frames".into())
}

pub(super) fn validate_chat_commands(records: &[FixtureFrame]) -> Result<(), String> {
    use gent_protocol::agent_chat_commands::{AgentChatCommandFrame as Frame, CommandOutcome};
    let frames: Vec<Frame> = canonical(records)?;
    let valid = frames.iter().all(|frame| frame.validate().is_ok());
    (valid
        && matches!(
            frames.as_slice(),
            [
                Frame::ReadCommandCatalog { .. },
                Frame::CommandCatalog { .. },
                Frame::InvokeCommand { .. },
                Frame::CommandInvoked {
                    outcome: CommandOutcome::Delivered { .. },
                    ..
                },
                Frame::InvokeCommand { .. },
                Frame::CommandInvoked {
                    outcome: CommandOutcome::IntentApplied { .. },
                    ..
                }
            ]
        ))
    .then_some(())
    .ok_or_else(|| {
        "must contain valid catalog read, catalog, and delivered and applied invocations".into()
    })
}

pub(super) fn validate_provider_auth(records: &[FixtureFrame]) -> Result<(), String> {
    use gent_protocol::ProviderAuthFrame;
    use gent_types::ProviderAuthLifecycle;
    let frames: Vec<ProviderAuthFrame> = canonical(records)?;
    let valid = frames.iter().all(|frame| frame.validate().is_ok());
    let lifecycle = |frame: &ProviderAuthFrame| match frame {
        ProviderAuthFrame::Status { status, .. }
        | ProviderAuthFrame::SelectionAccepted { status, .. } => Some(status.lifecycle),
        _ => None,
    };
    (valid
        && frames.get(1).and_then(lifecycle) == Some(ProviderAuthLifecycle::Checking)
        && matches!(
            frames.as_slice(),
            [
                ProviderAuthFrame::StatusRequest { .. },
                ProviderAuthFrame::Status { .. },
                ProviderAuthFrame::StatusRequest { .. },
                ProviderAuthFrame::Status { .. },
                ProviderAuthFrame::LoginRequest { .. },
                ProviderAuthFrame::AskTool { .. },
                ProviderAuthFrame::SelectMethod { .. },
                ProviderAuthFrame::SelectionAccepted { .. },
                ProviderAuthFrame::Cancel { .. },
                ProviderAuthFrame::Status { .. }
            ]
        ))
    .then_some(())
    .ok_or_else(|| {
        "must contain valid checking, status, login, selection, and cancel frames".into()
    })
}

pub(super) fn validate_goal(records: &[FixtureFrame]) -> Result<(), String> {
    use gent_protocol::GoalFrame;
    let frames: Vec<GoalFrame> = canonical(records)?;
    let valid = frames.iter().all(|frame| frame.validate().is_ok());
    (valid
        && matches!(
            frames.as_slice(),
            [
                GoalFrame::Set { .. },
                GoalFrame::Goal { .. },
                GoalFrame::Pause { .. },
                GoalFrame::Resume { .. },
                GoalFrame::Clear { .. },
                GoalFrame::Read { .. },
                GoalFrame::Report { .. },
                GoalFrame::Rejected { .. }
            ]
        ))
    .then_some(())
    .ok_or_else(|| {
        "must contain valid set, goal, control, read, report, and rejected frames".into()
    })
}

pub(super) fn canonical<T>(records: &[FixtureFrame]) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    records
        .iter()
        .map(|record| {
            let frame: T =
                serde_json::from_value(record.frame.clone()).map_err(|error| error.to_string())?;
            let canonical = serde_json::to_value(&frame).map_err(|error| error.to_string())?;
            if canonical != record.frame {
                return Err("frame does not match its canonical public JSON shape".into());
            }
            if encoded_frame_hex(&frame)? != record.wire_hex {
                return Err("frame wireHex does not match canonical u32be JSON bytes".into());
            }
            Ok(frame)
        })
        .collect()
}
