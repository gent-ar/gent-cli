use std::collections::{BTreeMap, BTreeSet};

use gent_types::ConversationActivityFact;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuedPrompt {
    pub(crate) message_id: String,
    pub(crate) run_id: String,
    pub(crate) turn_id: String,
    pub(crate) cursor: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) delivery_unconfirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) steer_receipt_id: Option<String>,
}

impl QueuedPrompt {
    pub(crate) fn can_steer(&self) -> bool {
        self.steer_receipt_id.is_none() && !self.delivery_unconfirmed
    }
}

pub(crate) fn queued_prompts(facts: &[ConversationActivityFact]) -> Vec<QueuedPrompt> {
    let mut ordered = facts.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|fact| fact.scope().cursor);
    let mut requested = BTreeMap::new();
    for fact in ordered {
        match fact {
            ConversationActivityFact::PromptSteerRequested {
                message_id,
                receipt_id,
                ..
            } => {
                requested.insert(message_id, receipt_id);
            }
            ConversationActivityFact::PromptSteerRejected {
                message_id,
                receipt_id,
                ..
            } if requested.get(message_id) == Some(&receipt_id) => {
                requested.remove(message_id);
            }
            _ => {}
        }
    }
    let settled = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptReleased { message_id, .. }
            | ConversationActivityFact::PromptCanceled { message_id, .. }
            | ConversationActivityFact::PromptWithdrawn { message_id, .. }
            | ConversationActivityFact::PromptSteered { message_id, .. } => {
                Some(message_id.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let unconfirmed = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptSteerRejected {
                message_id, reason, ..
            } if reason == "deliveryUnprovable" => Some(message_id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut queued = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptQueued {
                scope, message_id, ..
            } if !settled.contains(message_id.as_str()) && seen.insert(message_id.clone()) => {
                Some(QueuedPrompt {
                    message_id: message_id.clone(),
                    run_id: scope.run_id.clone(),
                    turn_id: scope.turn_id.clone(),
                    cursor: scope.cursor,
                    delivery_unconfirmed: unconfirmed.contains(message_id.as_str()),
                    steer_receipt_id: requested.get(message_id).map(|receipt| (*receipt).clone()),
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    queued.sort_by_key(|prompt| prompt.cursor);
    queued
}

#[cfg(test)]
#[path = "prompt_queue_tests.rs"]
mod tests;
