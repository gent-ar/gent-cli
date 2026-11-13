use gent_protocol::{AgentChatIntentFrame as Frame, AgentChatSubscriptionEnd};

#[path = "agent_chat_transport_reply_links.rs"]
mod links;
#[path = "agent_chat_transport_reply_shapes.rs"]
mod shapes;

pub(super) fn validate_replies(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    match request {
        Frame::Subscribe {
            request_id,
            after_cursor,
            ..
        } => validate_subscription(request_id, *after_cursor, replies),
        Frame::CreateConversation { .. } => shapes::created(request, replies),
        Frame::ImportTranscript { .. } => shapes::transcript_imported(request, replies),
        Frame::SwitchSelection { .. } => shapes::switched(request, replies),
        Frame::SendPrompt { .. }
        | Frame::ContinueFromSavedHistory { .. }
        | Frame::QueuePrompt { .. }
        | Frame::SendPromptWithTools { .. }
        | Frame::QueuePromptWithTools { .. }
        | Frame::Decision { .. } => shapes::accepted(request, replies),
        Frame::CancelQueuedPrompt { .. } => shapes::queued_prompt_canceled(request, replies),
        Frame::ArchiveConversation { .. } => shapes::conversation_archived(request, replies),
        Frame::SteerPromptForTurn(_) => shapes::atomic_steer_accepted(request, replies),
        Frame::SteerQueuedPrompt { .. } | Frame::SteerQueuedPromptForTurn { .. } => {
            shapes::queued_prompt_steered(request, replies)
        }
        Frame::Interrupt { .. } => shapes::interrupted(request, replies),
        Frame::WarmProvider(warm) => match replies {
            [Frame::ProviderWarm(result)]
                if result.request_id == warm.request_id
                    && result.conversation_id == warm.conversation_id
                    && result.run_id == warm.run_id =>
            {
                Ok(())
            }
            _ => Err("a provider warm-up requires one matching result"),
        },
        Frame::ForkConversation { .. } => shapes::forked(request, replies),
        Frame::CreateLinkedConversation { .. } => {
            links::linked_conversation_created(request, replies)
        }
        Frame::SendToConversation { .. } => links::conversation_message_delivered(request, replies),
        Frame::WaitForConversations { .. } => links::conversation_wait_settled(request, replies),
        Frame::ListLinkedConversations { .. } => links::conversation_links(request, replies),
        Frame::SubscriptionEvent { .. }
        | Frame::SubscriptionEnded { .. }
        | Frame::Created { .. }
        | Frame::TranscriptImported { .. }
        | Frame::Switched { .. }
        | Frame::Accepted { .. }
        | Frame::Interrupted { .. }
        | Frame::ProviderWarm(_)
        | Frame::QueuedPromptCanceled { .. }
        | Frame::ConversationArchived { .. }
        | Frame::QueuedPromptSteered { .. }
        | Frame::Forked { .. }
        | Frame::LinkedConversationCreated { .. }
        | Frame::ConversationMessageDelivered { .. }
        | Frame::ConversationWaitSettled { .. }
        | Frame::ConversationLinks { .. } => Err("agent chat response frames are server-only"),
    }
}

fn validate_subscription(
    request_id: &gent_types::AgentChatRequestId,
    after_cursor: u64,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Some((last, events)) = replies.split_last() else {
        return Err("a chat subscription must end explicitly");
    };
    if !matches!(last, Frame::SubscriptionEnded { request_id: reply_id, reason: AgentChatSubscriptionEnd::ServerClosing | AgentChatSubscriptionEnd::ResyncRequired } if reply_id == request_id)
    {
        return Err("a chat subscription must end with its matching terminal frame");
    }
    let mut cursor = after_cursor;
    for event in events {
        let Frame::SubscriptionEvent {
            request_id: reply_id,
            event,
        } = event
        else {
            return Err("chat subscription replies may contain only events before termination");
        };
        if reply_id != request_id || event.cursor <= cursor {
            return Err("chat subscription event correlation or cursor is invalid");
        }
        cursor = event.cursor;
    }
    Ok(())
}
