use gent_ports::{ConversationContentReader, TranscriptLedger};
use gent_types::{
    AgentChatConversationId, AgentChatRunContext, ContextPolicy, FrozenConversationContext,
};

use super::{
    ConversationContextArtifactService, ConversationContextRequest, MAX_ENTRIES, PAGE_LIMIT,
    invariant,
};
use crate::RuntimeError;

impl<L: ConversationContentReader + TranscriptLedger> ConversationContextArtifactService<L> {
    pub fn project_run_before_message(
        &self,
        run: &AgentChatRunContext,
        message_id: &str,
    ) -> Result<FrozenConversationContext, RuntimeError> {
        let ordinal = self.message_ordinal(&run.conversation_id, message_id)?;
        let preserve = run.context_policy == ContextPolicy::Preserve;
        let inherited_through = if preserve {
            run.context_through_ordinal
        } else {
            0
        };
        self.preserved(
            &ConversationContextRequest {
                conversation_id: run.conversation_id.clone(),
                context_policy: ContextPolicy::Preserve,
                context_through_ordinal: ordinal.saturating_sub(1),
            },
            &|entry| entry.ordinal <= inherited_through || entry.run_id == run.run_id.0,
            preserve,
        )
    }

    pub(super) fn message_ordinal(
        &self,
        conversation_id: &AgentChatConversationId,
        message_id: &str,
    ) -> Result<u64, RuntimeError> {
        let mut before = None;
        let mut scanned = 0usize;
        loop {
            let page =
                self.reader
                    .read_conversation_content(&conversation_id.0, before, PAGE_LIMIT)?;
            if page.conversation_id != conversation_id.0 {
                return Err(invariant(
                    "conversation context belongs to another conversation",
                ));
            }
            scanned = scanned.saturating_add(page.entries.len());
            if scanned > MAX_ENTRIES {
                return Err(invariant("conversation context exceeds entry bound"));
            }
            if let Some(entry) = page
                .entries
                .iter()
                .find(|entry| entry.message_id == message_id)
            {
                return Ok(entry.ordinal);
            }
            before = page
                .next_before
                .map(|cursor| cursor.ordinal_for(&conversation_id.0))
                .transpose()
                .map_err(|_| invariant("conversation context cursor is invalid"))?;
            if before.is_none() {
                return Err(invariant("conversation context message is unavailable"));
            }
        }
    }
}
