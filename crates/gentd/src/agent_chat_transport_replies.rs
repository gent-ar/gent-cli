use gent_protocol::{AgentChatIntentFrame, AgentChatSubscriptionEnd};

pub(super) fn validate_replies(
    request: &AgentChatIntentFrame,
    replies: &[AgentChatIntentFrame],
) -> Result<(), &'static str> {
    match request {
        AgentChatIntentFrame::Subscribe {
            request_id,
            after_cursor,
            ..
        } => validate_subscription(request_id, *after_cursor, replies),
        AgentChatIntentFrame::CreateConversation {
            request_id,
            receipt_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Created { request_id: reply_id, receipt, conversation_id, run_id }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && !conversation_id.0.is_empty() && !run_id.0.is_empty()
        )
        .then_some(())
        .ok_or("conversation creation requires one matching durable result"),
        AgentChatIntentFrame::ImportTranscript {
            request_id,
            conversation_id,
            run_id,
            entries,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::TranscriptImported {
                request_id: reply_id,
                conversation_id: reply_conversation,
                run_id: reply_run,
                imported_count,
            }] if reply_id == request_id
                && reply_conversation == conversation_id
                && reply_run == run_id
                && usize::from(*imported_count) == entries.len()
        )
        .then_some(())
        .ok_or("historical transcript import requires one matching result"),
        AgentChatIntentFrame::SwitchSelection {
            request_id,
            receipt_id,
            conversation_id,
            parent_run_id,
            context_policy,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Switched { request_id: reply_id, receipt, conversation_id: reply_conversation, parent_run_id: reply_parent, run_id, context_policy: reply_policy, context_through_ordinal, .. }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && reply_conversation == conversation_id && reply_parent == parent_run_id
                    && reply_policy == context_policy && !run_id.0.is_empty()
                    && (*context_policy != gent_types::ContextPolicy::Clear
                        || *context_through_ordinal == 0)
        )
        .then_some(())
        .ok_or("selection switching requires one matching durable child run"),
        AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::ContinueFromSavedHistory {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::QueuePrompt {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::SendPromptWithTools {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::QueuePromptWithTools {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::Decision {
            request_id,
            receipt_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Accepted { request_id: reply_id, receipt, .. }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
        )
        .then_some(())
        .ok_or("a chat command requires one matching accepted receipt"),
        AgentChatIntentFrame::CancelQueuedPrompt {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
        } => matches!(
            replies,
            [AgentChatIntentFrame::QueuedPromptCanceled { request_id: reply_id, receipt, conversation_id: reply_conversation, message_id: reply_message }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && reply_conversation == conversation_id && reply_message == message_id
        )
        .then_some(())
        .ok_or("a queued prompt cancellation requires one matching durable result"),
        AgentChatIntentFrame::SteerQueuedPrompt { request_id, receipt_id, conversation_id, message_id } => matches!(
            replies,
            [AgentChatIntentFrame::QueuedPromptSteered { request_id: reply_id, receipt, conversation_id: reply_conversation, message_id: reply_message }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id && reply_conversation == conversation_id && reply_message == message_id
        ).then_some(()).ok_or("a queued prompt steer requires one matching durable result"),
        AgentChatIntentFrame::Interrupt { request_id, receipt_id, conversation_id, run_id } => matches!(
            replies,
            [AgentChatIntentFrame::Interrupted { request_id: reply_id, receipt, conversation_id: reply_conversation, run_id: reply_run }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id && reply_conversation == conversation_id && reply_run == run_id
        ).then_some(()).ok_or("an interrupt requires one matching durable result"),
        AgentChatIntentFrame::ForkConversation {
            request_id,
            receipt_id,
            source_conversation_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Forked { request_id: reply_id, receipt, source_conversation_id: reply_source, conversation_id, run_id }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && reply_source == source_conversation_id
                    && !conversation_id.0.is_empty() && !run_id.0.is_empty()
        )
        .then_some(())
        .ok_or("conversation forking requires one matching durable result"),
        AgentChatIntentFrame::CreateLinkedConversation {
            request_id,
            receipt_id,
            parent_conversation_id,
            label,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::LinkedConversationCreated { request_id: reply_id, receipt, parent_conversation_id: reply_parent, conversation_id, run_id, label: reply_label }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && reply_parent == parent_conversation_id && reply_label == label
                    && !conversation_id.0.is_empty() && !run_id.0.is_empty()
                    && conversation_id != parent_conversation_id
        )
        .then_some(())
        .ok_or("creating a linked conversation requires one matching durable result"),
        AgentChatIntentFrame::SendToConversation {
            request_id,
            receipt_id,
            from_conversation_id,
            target_conversation_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::ConversationMessageDelivered { request_id: reply_id, receipt, from_conversation_id: reply_from, target_conversation_id: reply_target, message_id, .. }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && reply_from == from_conversation_id && reply_target == target_conversation_id
                    && !message_id.is_empty()
        )
        .then_some(())
        .ok_or("a cross-conversation message requires one matching durable delivery"),
        AgentChatIntentFrame::WaitForConversations {
            request_id,
            from_conversation_id,
            conversation_ids,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::ConversationWaitSettled { request_id: reply_id, from_conversation_id: reply_from, results, .. }]
                if reply_id == request_id && reply_from == from_conversation_id
                    && results.len() == conversation_ids.len()
                    && results.iter().zip(conversation_ids).all(|(result, requested)| result.target.conversation_id == requested.0)
        )
        .then_some(())
        .ok_or("a conversation wait requires one honest result per requested conversation"),
        AgentChatIntentFrame::ListLinkedConversations {
            request_id,
            conversation_id,
        } => matches!(
            replies,
            [AgentChatIntentFrame::ConversationLinks { request_id: reply_id, conversation_id: reply_conversation, .. }]
                if reply_id == request_id && reply_conversation == conversation_id
        )
        .then_some(())
        .ok_or("listing linked conversations requires one matching thread"),
        AgentChatIntentFrame::SubscriptionEvent { .. }
        | AgentChatIntentFrame::SubscriptionEnded { .. }
        | AgentChatIntentFrame::Created { .. }
        | AgentChatIntentFrame::TranscriptImported { .. }
        | AgentChatIntentFrame::Switched { .. }
        | AgentChatIntentFrame::Accepted { .. }
        | AgentChatIntentFrame::Interrupted { .. }
        | AgentChatIntentFrame::QueuedPromptCanceled { .. }
        | AgentChatIntentFrame::QueuedPromptSteered { .. }
        | AgentChatIntentFrame::Forked { .. }
        | AgentChatIntentFrame::LinkedConversationCreated { .. }
        | AgentChatIntentFrame::ConversationMessageDelivered { .. }
        | AgentChatIntentFrame::ConversationWaitSettled { .. }
        | AgentChatIntentFrame::ConversationLinks { .. } => {
            Err("agent chat response frames are server-only")
        }
    }
}

fn validate_subscription(
    request_id: &gent_types::AgentChatRequestId,
    after_cursor: u64,
    replies: &[AgentChatIntentFrame],
) -> Result<(), &'static str> {
    let Some((last, events)) = replies.split_last() else {
        return Err("a chat subscription must end explicitly");
    };
    if !matches!(last, AgentChatIntentFrame::SubscriptionEnded { request_id: reply_id, reason: AgentChatSubscriptionEnd::ServerClosing | AgentChatSubscriptionEnd::ResyncRequired } if reply_id == request_id)
    {
        return Err("a chat subscription must end with its matching terminal frame");
    }
    let mut cursor = after_cursor;
    for event in events {
        let AgentChatIntentFrame::SubscriptionEvent {
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
