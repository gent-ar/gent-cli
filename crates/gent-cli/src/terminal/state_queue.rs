use gent_types::{ConversationActivityFact, NormalizedTranscriptKind, TurnTerminalCause};

use super::{UiEffect, UiRequest, UiState};
use crate::prompt_queue::{QueuedPrompt, queued_prompts};

impl UiState {
    #[must_use]
    pub(crate) fn turn_active(&self) -> bool {
        self.awaiting_turn()
            || self
                .selected_status()
                .is_some_and(|status| status.runs.iter().any(|run| run.active_turn_id.is_some()))
    }

    #[must_use]
    pub(crate) fn selected_queue(&self) -> Vec<QueuedPrompt> {
        queued_prompts(self.selected_activity())
    }

    pub(super) fn steer_queued(&mut self) -> UiEffect {
        let queue = self.selected_queue();
        match self.selected().map(|item| item.conversation_id.clone()) {
            Some(conversation_id) if !queue.is_empty() => {
                UiEffect::Request(UiRequest::SteerQueued {
                    conversation_id,
                    message_ids: queue.into_iter().map(|prompt| prompt.message_id).collect(),
                })
            }
            _ => {
                self.notice = Some(
                    "No prompts are queued. Press Enter while Gent is working to queue one.".into(),
                );
                UiEffect::Continue
            }
        }
    }

    pub(super) fn interrupt(&mut self) -> UiEffect {
        let (Some(conversation_id), Some(run_id)) = (
            self.selected().map(|item| item.conversation_id.clone()),
            self.parent_run_id.clone(),
        ) else {
            self.notice = Some("No active run is available to cancel.".into());
            return UiEffect::Continue;
        };
        UiEffect::Request(UiRequest::Interrupt {
            conversation_id,
            run_id,
        })
    }

    pub(super) fn continue_from_history(&mut self) -> UiEffect {
        let lost_turn = self
            .selected_activity()
            .iter()
            .rev()
            .find_map(|fact| match fact {
                ConversationActivityFact::Terminal {
                    scope,
                    cause: Some(TurnTerminalCause::ProviderSessionUnavailable),
                    ..
                } => Some(scope.turn_id.clone()),
                _ => None,
            });
        let message_id = lost_turn.and_then(|turn_id| {
            self.view
                .as_ref()?
                .transcript()
                .iter()
                .find(|event| {
                    event.turn_id == turn_id && event.kind == NormalizedTranscriptKind::UserMessage
                })?
                .event_id
                .strip_prefix("user:")
                .map(str::to_owned)
        });
        let (Some(conversation_id), Some(message_id)) = (
            self.selected().map(|item| item.conversation_id.clone()),
            message_id,
        ) else {
            self.notice = Some("No turn here lost its provider session.".into());
            return UiEffect::Continue;
        };
        self.input.clear();
        UiEffect::Request(UiRequest::ContinueFromHistory {
            conversation_id,
            message_id,
        })
    }

    pub(super) fn cancel_queued(&mut self) -> UiEffect {
        let newest = self.selected_queue().pop();
        let (Some(conversation_id), Some(prompt)) = (
            self.selected().map(|item| item.conversation_id.clone()),
            newest,
        ) else {
            self.notice = Some("No prompts are queued.".into());
            return UiEffect::Continue;
        };
        UiEffect::Request(UiRequest::CancelQueued {
            conversation_id,
            message_id: prompt.message_id,
        })
    }
}
