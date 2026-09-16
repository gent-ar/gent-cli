use super::shapes::MISMATCHED_REQUEST;
use gent_protocol::AgentChatIntentFrame as Frame;

pub(super) fn linked_conversation_created(
    request: &Frame,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Frame::CreateLinkedConversation {
        request_id,
        receipt_id,
        parent_conversation_id,
        label,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::LinkedConversationCreated { request_id: reply_id, receipt, parent_conversation_id: reply_parent, conversation_id, run_id, label: reply_label }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_parent == parent_conversation_id && reply_label == label
                && !conversation_id.0.is_empty() && !run_id.0.is_empty()
                && conversation_id != parent_conversation_id
    )
    .then_some(())
    .ok_or("creating a linked conversation requires one matching durable result")
}

pub(super) fn conversation_message_delivered(
    request: &Frame,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Frame::SendToConversation {
        request_id,
        receipt_id,
        from_conversation_id,
        target_conversation_id,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::ConversationMessageDelivered { request_id: reply_id, receipt, from_conversation_id: reply_from, target_conversation_id: reply_target, message_id, .. }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_from == from_conversation_id && reply_target == target_conversation_id
                && !message_id.is_empty()
    )
    .then_some(())
    .ok_or("a cross-conversation message requires one matching durable delivery")
}

pub(super) fn conversation_wait_settled(
    request: &Frame,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Frame::WaitForConversations {
        request_id,
        from_conversation_id,
        conversation_ids,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::ConversationWaitSettled { request_id: reply_id, from_conversation_id: reply_from, results, .. }]
            if reply_id == request_id && reply_from == from_conversation_id
                && results.len() == conversation_ids.len()
                && results.iter().zip(conversation_ids).all(|(result, requested)| result.target.conversation_id == requested.0)
    )
    .then_some(())
    .ok_or("a conversation wait requires one honest result per requested conversation")
}

pub(super) fn conversation_links(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::ListLinkedConversations {
        request_id,
        conversation_id,
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::ConversationLinks { request_id: reply_id, conversation_id: reply_conversation, .. }]
            if reply_id == request_id && reply_conversation == conversation_id
    )
    .then_some(())
    .ok_or("listing linked conversations requires one matching thread")
}
