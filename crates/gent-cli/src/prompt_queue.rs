use std::collections::BTreeSet;

use gent_types::ConversationActivityFact;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuedPrompt {
    pub(crate) message_id: String,
    pub(crate) run_id: String,
    pub(crate) turn_id: String,
    pub(crate) cursor: u64,
}

pub(crate) fn queued_prompts(facts: &[ConversationActivityFact]) -> Vec<QueuedPrompt> {
    let settled = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptReleased { message_id, .. }
            | ConversationActivityFact::PromptCanceled { message_id, .. }
            | ConversationActivityFact::PromptSteered { message_id, .. } => {
                Some(message_id.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut queued = facts
        .iter()
        .filter_map(|fact| match fact {
            ConversationActivityFact::PromptQueued { scope, message_id }
                if !settled.contains(message_id.as_str()) && seen.insert(message_id.clone()) =>
            {
                Some(QueuedPrompt {
                    message_id: message_id.clone(),
                    run_id: scope.run_id.clone(),
                    turn_id: scope.turn_id.clone(),
                    cursor: scope.cursor,
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
