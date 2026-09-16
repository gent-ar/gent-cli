use gent_ports::ConversationLinkLedger;
use gent_protocol::{
    AgentChatIntentFrame,
    conversation_links::{
        ConversationLinkRequestError, is_conversation_link_frame,
        validate_conversation_link_request,
    },
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, ConversationActivityFact,
    ConversationActivityScope, ConversationLink, ConversationThreadState, HostEpoch,
    LinkedConversationSummary, LinkedConversations, MAX_CONVERSATION_LABEL_BYTES, ReceiptId,
};

use super::RuntimeFacade;
use crate::{agent_chat_intent_error::AgentChatIntentError, api::RuntimeApi};

#[path = "runtime_facade_conversation_links_delivery.rs"]
mod delivery;

impl RuntimeFacade {
    pub(super) fn conversation_link_intent(
        &self,
        frame: &AgentChatIntentFrame,
    ) -> Result<Option<Vec<AgentChatIntentFrame>>, AgentChatIntentError> {
        if !is_conversation_link_frame(frame) {
            return Ok(None);
        }
        validate_conversation_link_request(frame).map_err(link_error)?;
        match frame {
            AgentChatIntentFrame::CreateLinkedConversation {
                request_id,
                receipt_id,
                parent_conversation_id,
                workspace_path,
                selection,
                label,
                prompt,
                origin_tool_use_id,
            } => self
                .create_linked_conversation(CreateLinkedInput {
                    request_id: request_id.clone(),
                    receipt_id: receipt_id.clone(),
                    parent_conversation_id: parent_conversation_id.clone(),
                    workspace_path: workspace_path.clone(),
                    selection: selection.clone(),
                    label: label.clone(),
                    prompt: prompt.clone(),
                    origin_tool_use_id: origin_tool_use_id.clone(),
                })
                .map(Some),
            AgentChatIntentFrame::SendToConversation {
                request_id,
                receipt_id,
                from_conversation_id,
                target_conversation_id,
                message,
            } => self
                .send_to_conversation(
                    request_id,
                    receipt_id,
                    from_conversation_id,
                    target_conversation_id,
                    message,
                )
                .map(Some),
            AgentChatIntentFrame::WaitForConversations {
                request_id,
                from_conversation_id,
                conversation_ids,
                timeout_seconds,
            } => self
                .wait_for_conversations(
                    request_id,
                    from_conversation_id,
                    conversation_ids,
                    *timeout_seconds,
                )
                .map(Some),
            AgentChatIntentFrame::ListLinkedConversations {
                request_id,
                conversation_id,
            } => self
                .list_linked_conversations(request_id, conversation_id)
                .map(Some),
            _ => Err("conversation link response frames are server-only".into()),
        }
    }

    fn create_linked_conversation(
        &self,
        input: CreateLinkedInput,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let links = self.conversation_links()?;
        let parent = self.thread_state(&input.parent_conversation_id)?;
        let summary = self
            .agent_chat_reads
            .as_ref()
            .ok_or("agent-chat reads are unavailable")?
            .detail(&input.parent_conversation_id.0)?
            .summary;
        let workspace_path = input
            .workspace_path
            .clone()
            .or_else(|| summary.workspace_path.clone())
            .ok_or_else(|| link_error(ConversationLinkRequestError::UnknownConversation))?;
        let selection = input.selection.clone().unwrap_or(summary.selection);
        let created = self.agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(format!("linked-create-{}", input.request_id.0)),
            receipt_id: input.receipt_id.clone(),
            workspace_path: workspace_path.clone(),
            selection: Some(selection.clone()),
        })?;
        let Some(AgentChatIntentFrame::Created {
            receipt,
            conversation_id,
            run_id,
            ..
        }) = created.into_iter().next()
        else {
            return Err("linked conversation was not created".into());
        };
        let child = self.thread_state(&conversation_id)?;
        let host_epoch = self.host_epoch()?;
        let link = ConversationLink {
            parent_conversation_id: input.parent_conversation_id.0.clone(),
            child_conversation_id: conversation_id.0.clone(),
            label: input.label.clone(),
            created_run_id: parent.run_id.clone(),
            created_turn_id: parent.fact_turn_id(),
            created_at_unix_seconds: crate::startup::unix_seconds(),
        };
        links
            .record_conversation_link(
                &link,
                &[
                    ConversationActivityFact::ConversationCreated {
                        scope: scope_for(&parent, host_epoch),
                        child_conversation_id: conversation_id.0.clone(),
                        label: input.label.clone(),
                        workspace_path,
                        selection,
                        origin_tool_use_id: input.origin_tool_use_id.clone(),
                    },
                    ConversationActivityFact::CreatedByConversation {
                        scope: scope_for(&child, host_epoch),
                        parent_conversation_id: input.parent_conversation_id.0.clone(),
                        parent_run_id: parent.run_id.clone(),
                        label: self.conversation_label(&input.parent_conversation_id.0),
                    },
                ],
                &input.request_id.0,
            )
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?;
        if let Some(prompt) = input.prompt.clone() {
            self.agent_chat_intent(AgentChatIntentFrame::SendPrompt {
                request_id: AgentChatRequestId(format!("linked-prompt-{}", input.request_id.0)),
                receipt_id: ReceiptId(format!("linked-prompt-receipt-{}", input.request_id.0)),
                conversation_id: conversation_id.clone(),
                text: prompt,
                attachment_ids: Vec::new(),
            })?;
        }
        Ok(vec![AgentChatIntentFrame::LinkedConversationCreated {
            request_id: input.request_id,
            receipt,
            parent_conversation_id: input.parent_conversation_id,
            conversation_id,
            run_id,
            label: input.label,
        }])
    }

    fn list_linked_conversations(
        &self,
        request_id: &AgentChatRequestId,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let links = self.conversation_links()?;
        self.thread_state(conversation_id)?;
        let parent = links
            .read_conversation_parent(&conversation_id.0)
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?
            .map(|link| self.summarize(&link.parent_conversation_id, link.label))
            .transpose()?;
        let children = links
            .read_conversation_children(&conversation_id.0)
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?
            .into_iter()
            .map(|link| self.summarize(&link.child_conversation_id, link.label))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(vec![AgentChatIntentFrame::ConversationLinks {
            request_id: request_id.clone(),
            conversation_id: conversation_id.clone(),
            links: LinkedConversations { parent, children },
        }])
    }

    fn summarize(
        &self,
        conversation_id: &str,
        label: String,
    ) -> Result<LinkedConversationSummary, AgentChatIntentError> {
        let state = self.thread_state(&AgentChatConversationId(conversation_id.to_owned()))?;
        Ok(LinkedConversationSummary {
            conversation_id: state.conversation_id,
            label,
            phase: state.phase,
            last_activity_unix_seconds: state.last_activity_unix_seconds,
        })
    }

    pub(super) fn conversation_label(&self, conversation_id: &str) -> String {
        if let Ok(Some(link)) = self
            .conversation_links
            .as_ref()
            .ok_or(())
            .and_then(|links| {
                links
                    .read_conversation_parent(conversation_id)
                    .map_err(|_| ())
            })
        {
            return link.label;
        }
        self.agent_chat_reads
            .as_ref()
            .and_then(|reads| reads.detail(conversation_id).ok())
            .and_then(|detail| detail.summary.title)
            .map(|title| gent_types::bounded_excerpt(title.trim(), MAX_CONVERSATION_LABEL_BYTES))
            .unwrap_or_default()
    }

    pub(super) fn conversation_links(&self) -> Result<&SqliteLedger, AgentChatIntentError> {
        self.conversation_links.as_ref().ok_or_else(|| {
            "conversation links are unavailable while gentd is observer-disabled".into()
        })
    }

    pub(super) fn thread_state(
        &self,
        conversation_id: &AgentChatConversationId,
    ) -> Result<ConversationThreadState, AgentChatIntentError> {
        self.conversation_links()?
            .read_conversation_thread_state(&conversation_id.0)
            .map_err(|error| AgentChatIntentError::from(error.to_string()))?
            .ok_or_else(|| link_error(ConversationLinkRequestError::UnknownConversation))
    }
}

pub(super) fn require_same_workspace(
    from: &ConversationThreadState,
    target: &ConversationThreadState,
) -> Result<(), AgentChatIntentError> {
    (from.workspace_id.is_some() && from.workspace_id == target.workspace_id)
        .then_some(())
        .ok_or_else(|| AgentChatIntentError {
            code: "conversationForeignWorkspace",
            message: "a conversation may only address conversations in its own workspace".into(),
        })
}

pub(super) fn scope_for(
    state: &ConversationThreadState,
    host_epoch: HostEpoch,
) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: state.conversation_id.clone(),
        run_id: state.run_id.clone(),
        turn_id: state.fact_turn_id(),
        host_epoch,
        cursor: 0,
    }
}

pub(super) fn link_error(error: ConversationLinkRequestError) -> AgentChatIntentError {
    AgentChatIntentError {
        code: error.code(),
        message: error.to_string(),
    }
}

struct CreateLinkedInput {
    request_id: AgentChatRequestId,
    receipt_id: ReceiptId,
    parent_conversation_id: AgentChatConversationId,
    workspace_path: Option<String>,
    selection: Option<gent_types::AgentChatSelection>,
    label: String,
    prompt: Option<String>,
    origin_tool_use_id: Option<String>,
}
