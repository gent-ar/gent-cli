use gent_types::{ConversationActivityFact, ConversationMessageDelivery};
use ratatui::style::Color;

use super::clip;

pub(super) fn text(fact: &ConversationActivityFact) -> (String, Color) {
    match fact {
        ConversationActivityFact::ConversationCreated {
            child_conversation_id,
            label,
            ..
        } => (
            format!("Chat created · {} · {}", label, clip(child_conversation_id)),
            Color::Magenta,
        ),
        ConversationActivityFact::CreatedByConversation {
            parent_conversation_id,
            label,
            ..
        } => (
            format!(
                "Created by chat · {} · {}",
                label,
                clip(parent_conversation_id)
            ),
            Color::Magenta,
        ),
        ConversationActivityFact::ConversationMessageSent {
            target_conversation_id,
            delivery,
            preview,
            ..
        } => (
            format!(
                "Message {} to chat {} · {}",
                match delivery {
                    ConversationMessageDelivery::Queued => "queued",
                    ConversationMessageDelivery::Steered => "steered",
                },
                clip(target_conversation_id),
                clip(preview)
            ),
            Color::Cyan,
        ),
        ConversationActivityFact::ConversationWaitSettled {
            targets, timed_out, ..
        } => (
            format!(
                "Waited on {} chat{} · {}",
                targets.len(),
                if targets.len() == 1 { "" } else { "s" },
                if *timed_out {
                    "timed out"
                } else {
                    "all settled"
                }
            ),
            if *timed_out {
                Color::Yellow
            } else {
                Color::Green
            },
        ),
        _ => (String::new(), Color::DarkGray),
    }
}
