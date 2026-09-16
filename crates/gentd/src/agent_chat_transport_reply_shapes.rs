use gent_protocol::AgentChatIntentFrame as Frame;
use gent_types::ContextPolicy;

pub(super) const MISMATCHED_REQUEST: &str =
    "agent chat reply validation received a mismatched request";

pub(super) fn created(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::CreateConversation {
        request_id,
        receipt_id,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::Created { request_id: reply_id, receipt, conversation_id, run_id }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && !conversation_id.0.is_empty() && !run_id.0.is_empty()
    )
    .then_some(())
    .ok_or("conversation creation requires one matching durable result")
}

pub(super) fn transcript_imported(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::ImportTranscript {
        request_id,
        conversation_id,
        run_id,
        entries,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::TranscriptImported {
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
    .ok_or("historical transcript import requires one matching result")
}

pub(super) fn switched(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::SwitchSelection {
        request_id,
        receipt_id,
        conversation_id,
        parent_run_id,
        context_policy,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::Switched { request_id: reply_id, receipt, conversation_id: reply_conversation, parent_run_id: reply_parent, run_id, context_policy: reply_policy, context_through_ordinal, .. }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id && reply_parent == parent_run_id
                && reply_policy == context_policy && !run_id.0.is_empty()
                && (*context_policy != ContextPolicy::Clear || *context_through_ordinal == 0)
    )
    .then_some(())
    .ok_or("selection switching requires one matching durable child run")
}

pub(super) fn accepted(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let (Frame::SendPrompt {
        request_id,
        receipt_id,
        ..
    }
    | Frame::ContinueFromSavedHistory {
        request_id,
        receipt_id,
        ..
    }
    | Frame::QueuePrompt {
        request_id,
        receipt_id,
        ..
    }
    | Frame::SendPromptWithTools {
        request_id,
        receipt_id,
        ..
    }
    | Frame::QueuePromptWithTools {
        request_id,
        receipt_id,
        ..
    }
    | Frame::Decision {
        request_id,
        receipt_id,
        ..
    }) = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::Accepted { request_id: reply_id, receipt, .. }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
    )
    .then_some(())
    .ok_or("a chat command requires one matching accepted receipt")
}

pub(super) fn queued_prompt_canceled(
    request: &Frame,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Frame::CancelQueuedPrompt {
        request_id,
        receipt_id,
        conversation_id,
        message_id,
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::QueuedPromptCanceled { request_id: reply_id, receipt, conversation_id: reply_conversation, message_id: reply_message }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id && reply_message == message_id
    )
    .then_some(())
    .ok_or("a queued prompt cancellation requires one matching durable result")
}

pub(super) fn queued_prompt_steered(
    request: &Frame,
    replies: &[Frame],
) -> Result<(), &'static str> {
    let Frame::SteerQueuedPrompt {
        request_id,
        receipt_id,
        conversation_id,
        message_id,
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::QueuedPromptSteered { request_id: reply_id, receipt, conversation_id: reply_conversation, message_id: reply_message }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id && reply_message == message_id
    )
    .then_some(())
    .ok_or("a queued prompt steer requires one matching durable result")
}

pub(super) fn interrupted(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::Interrupt {
        request_id,
        receipt_id,
        conversation_id,
        run_id,
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::Interrupted { request_id: reply_id, receipt, conversation_id: reply_conversation, run_id: reply_run }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id && reply_run == run_id
    )
    .then_some(())
    .ok_or("an interrupt requires one matching durable result")
}

pub(super) fn forked(request: &Frame, replies: &[Frame]) -> Result<(), &'static str> {
    let Frame::ForkConversation {
        request_id,
        receipt_id,
        source_conversation_id,
        ..
    } = request
    else {
        return Err(MISMATCHED_REQUEST);
    };
    matches!(
        replies,
        [Frame::Forked { request_id: reply_id, receipt, source_conversation_id: reply_source, conversation_id, run_id }]
            if reply_id == request_id && receipt.receipt_id == *receipt_id
                && reply_source == source_conversation_id
                && !conversation_id.0.is_empty() && !run_id.0.is_empty()
    )
    .then_some(())
    .ok_or("conversation forking requires one matching durable result")
}
