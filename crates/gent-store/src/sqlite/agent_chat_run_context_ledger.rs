//! Durable lookup for immutable agent-chat run context provenance.

use gent_ports::{AgentChatRunContextReader, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRunContext, AgentChatRunContextOrigin, AgentChatRunId,
    ContextPolicy,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::{SqliteLedger, queries::storage_error};

impl AgentChatRunContextReader for SqliteLedger {
    fn read_agent_chat_run_context(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatRunContext, LedgerError> {
        let connection = self.lock()?;
        context(&connection, conversation_id, run_id)
    }
}

fn context(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<AgentChatRunContext, LedgerError> {
    if let Some(context) = forked(connection, conversation_id, run_id)? {
        return Ok(context);
    }
    if let Some(context) = root(connection, conversation_id, run_id)? {
        return Ok(context);
    }
    if let Some(context) = selection_switch(connection, conversation_id, run_id)? {
        return Ok(context);
    }
    if let Some(context) = reviewed_plan(connection, conversation_id, run_id)? {
        return Ok(context);
    }
    if let Some(context) = checkpoint_restore(connection, conversation_id, run_id)? {
        return Ok(context);
    }
    Err(LedgerError::Invariant(
        "agent chat run has no durable context provenance".into(),
    ))
}

fn forked(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<Option<AgentChatRunContext>, LedgerError> {
    let row = connection
        .query_row(
            "SELECT f.context_through_ordinal FROM agent_chat_fork_receipts f JOIN runs r ON r.run_id = f.run_id WHERE f.conversation_id = ?1 AND f.run_id = ?2 AND r.conversation_id = f.conversation_id",
            params![conversation_id.0, run_id.0],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(row.map(|ordinal| {
        value(
            conversation_id,
            run_id,
            AgentChatRunContextOrigin::Forked,
            ContextPolicy::Preserve,
            ordinal,
        )
    }))
}

fn root(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<Option<AgentChatRunContext>, LedgerError> {
    let found = connection
        .query_row(
            "SELECT 1 FROM agent_chat_conversations c JOIN runs r ON r.run_id = c.root_run_id WHERE c.conversation_id = ?1 AND r.run_id = ?2 AND r.conversation_id = c.conversation_id",
            params![conversation_id.0, run_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(found.map(|()| {
        value(
            conversation_id,
            run_id,
            AgentChatRunContextOrigin::Root,
            ContextPolicy::Preserve,
            0,
        )
    }))
}

fn selection_switch(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<Option<AgentChatRunContext>, LedgerError> {
    let row = connection
        .query_row(
            "SELECT s.context_policy, s.context_through_ordinal FROM agent_chat_selection_switch_receipts s JOIN runs r ON r.run_id = s.run_id WHERE s.conversation_id = ?1 AND s.run_id = ?2 AND r.conversation_id = s.conversation_id",
            params![conversation_id.0, run_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    row.map(|(policy, ordinal)| {
        decode(
            conversation_id,
            run_id,
            AgentChatRunContextOrigin::SelectionSwitch,
            &policy,
            ordinal,
        )
    })
    .transpose()
}

fn reviewed_plan(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<Option<AgentChatRunContext>, LedgerError> {
    let row = connection
        .query_row(
            "SELECT a.context_policy, a.context_through_ordinal FROM reviewed_plan_approval_receipts a JOIN runs r ON r.run_id = a.implementation_run_id WHERE r.conversation_id = ?1 AND a.implementation_run_id = ?2",
            params![conversation_id.0, run_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    row.map(|(policy, ordinal)| {
        decode(
            conversation_id,
            run_id,
            AgentChatRunContextOrigin::ReviewedPlan,
            &policy,
            ordinal,
        )
    })
    .transpose()
}

fn checkpoint_restore(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<Option<AgentChatRunContext>, LedgerError> {
    let row = connection
        .query_row(
            "SELECT c.visible_through_ordinal FROM agent_chat_checkpoint_restore_receipts c JOIN runs r ON r.run_id = c.run_id WHERE c.conversation_id = ?1 AND c.run_id = ?2 AND r.conversation_id = c.conversation_id",
            params![conversation_id.0, run_id.0],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(row.map(|ordinal| {
        value(
            conversation_id,
            run_id,
            AgentChatRunContextOrigin::CheckpointRestore,
            ContextPolicy::Preserve,
            ordinal,
        )
    }))
}

fn decode(
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    origin: AgentChatRunContextOrigin,
    policy: &str,
    ordinal: u64,
) -> Result<AgentChatRunContext, LedgerError> {
    let policy = match policy {
        "preserve" => ContextPolicy::Preserve,
        "clear" if ordinal == 0 => ContextPolicy::Clear,
        _ => {
            return Err(LedgerError::Storage(
                "agent chat run context policy is invalid".into(),
            ));
        }
    };
    Ok(value(conversation_id, run_id, origin, policy, ordinal))
}

fn value(
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    origin: AgentChatRunContextOrigin,
    context_policy: ContextPolicy,
    context_through_ordinal: u64,
) -> AgentChatRunContext {
    AgentChatRunContext {
        conversation_id: conversation_id.clone(),
        run_id: run_id.clone(),
        origin,
        context_policy,
        context_through_ordinal,
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{
        AgentChatCheckpointLedger, AgentChatForkLedger, AgentChatLedger, AgentChatPromptLedger,
        AgentChatRunContextReader, AgentChatSelectionLedger, AgentChatWorkspaceLedger,
    };
    use gent_types::{
        AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatConversationCreate,
        AgentChatConversationId, AgentChatEffort, AgentChatFork, AgentChatMode,
        AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
        AgentChatRunContextOrigin, AgentChatRunId, AgentChatSelection, AgentChatSelectionSwitch,
        ContextPolicy, HostEpoch, ReceiptId, WorkspaceRecord,
    };

    use super::SqliteLedger;

    #[test]
    fn a_checkpoint_restore_run_carries_the_checkpoint_ordinal_as_its_boundary() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = AgentChatConversationId("conversation".into());
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("create-receipt".into()),
                    idempotency_key: "create-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    run_id: AgentChatRunId("root".into()),
                    selection: selection(AgentChatProvider::Claude),
                },
                &WorkspaceRecord {
                    workspace_id: "workspace".into(),
                    canonical_path: "/workspace".into(),
                },
            )
            .unwrap();
        let checkpoint = ledger
            .save_file_checkpoint(
                &AgentChatCheckpointCapture {
                    request_id: AgentChatRequestId("capture-request".into()),
                    receipt_id: ReceiptId("capture-receipt".into()),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    run_id: AgentChatRunId("root".into()),
                    message_ordinal: 3,
                    created_at_unix_ms: 1000,
                    files: vec![],
                },
                "checkpoint-a",
                "capture-idempotency",
                &[],
                25,
            )
            .unwrap();
        let restored = ledger
            .restore_file_checkpoint(
                &AgentChatCheckpointRestore {
                    request_id: AgentChatRequestId("restore-request".into()),
                    receipt_id: ReceiptId("restore-receipt".into()),
                    host_epoch: HostEpoch(1),
                    conversation_id: conversation.clone(),
                    checkpoint_id: checkpoint.checkpoint_id,
                    restore_files: false,
                    restore_files_confirmation: None,
                },
                "restore-idempotency",
                &AgentChatRunId("restored".into()),
            )
            .unwrap();
        let context = ledger
            .read_agent_chat_run_context(&conversation, &restored.run_id)
            .unwrap();
        assert_eq!(context.origin, AgentChatRunContextOrigin::CheckpointRestore);
        assert_eq!(context.context_policy, ContextPolicy::Preserve);
        assert_eq!(context.context_through_ordinal, 3);
    }

    #[test]
    fn a_forked_root_run_carries_the_copied_message_count_as_its_boundary() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let source = AgentChatConversationId("source".into());
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("create-receipt".into()),
                    idempotency_key: "create-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: source.clone(),
                    run_id: AgentChatRunId("source-root".into()),
                    selection: selection(AgentChatProvider::Claude),
                },
                &WorkspaceRecord {
                    workspace_id: "workspace".into(),
                    canonical_path: "/workspace".into(),
                },
            )
            .unwrap();
        let saved = ledger
            .save_agent_chat_prompt(&AgentChatPromptCreate {
                request_id: AgentChatRequestId("prompt-request".into()),
                receipt_id: ReceiptId("prompt-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: source.clone(),
                disposition: AgentChatPromptDisposition::Send,
                attachment_ids: vec![],
                tool_source_ids: vec![],
                text: "hello".into(),
            })
            .unwrap();
        let forked = ledger
            .fork_agent_chat_conversation(
                &AgentChatFork {
                    request_id: AgentChatRequestId("fork-request".into()),
                    receipt_id: ReceiptId("fork-receipt".into()),
                    host_epoch: HostEpoch(1),
                    source_conversation_id: source,
                    fork_through_message_id: saved.message.message_id,
                },
                &AgentChatConversationId("forked".into()),
                &AgentChatRunId("forked-root".into()),
            )
            .unwrap();
        let context = ledger
            .read_agent_chat_run_context(&forked.conversation_id, &forked.run_id)
            .unwrap();
        assert_eq!(context.origin, AgentChatRunContextOrigin::Forked);
        assert_eq!(context.context_policy, ContextPolicy::Preserve);
        assert_eq!(context.context_through_ordinal, 1);
    }

    #[test]
    fn root_and_selection_children_have_exact_context_provenance() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let conversation = AgentChatConversationId("conversation".into());
        let root = AgentChatRunId("root".into());
        ledger
            .create_agent_chat_conversation(&AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt".into()),
                idempotency_key: "create-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                run_id: root.clone(),
                selection: selection(AgentChatProvider::Codex),
            })
            .unwrap();
        let root_context = ledger
            .read_agent_chat_run_context(&conversation, &root)
            .unwrap();
        assert_eq!(root_context.origin, AgentChatRunContextOrigin::Root);
        assert_eq!(root_context.context_policy, ContextPolicy::Preserve);
        assert_eq!(root_context.context_through_ordinal, 0);

        let switched = AgentChatRunId("switched".into());
        ledger
            .switch_agent_chat_selection(&AgentChatSelectionSwitch {
                receipt_id: ReceiptId("switch-receipt".into()),
                idempotency_key: "switch-key".into(),
                host_epoch: HostEpoch(1),
                conversation_id: conversation.clone(),
                parent_run_id: root,
                run_id: switched.clone(),
                selection: selection(AgentChatProvider::Claude),
                context_policy: ContextPolicy::Clear,
            })
            .unwrap();
        assert_eq!(
            ledger
                .read_agent_chat_run_context(&conversation, &switched)
                .unwrap(),
            gent_types::AgentChatRunContext {
                conversation_id: conversation,
                run_id: switched,
                origin: AgentChatRunContextOrigin::SelectionSwitch,
                context_policy: ContextPolicy::Clear,
                context_through_ordinal: 0,
            }
        );
    }

    fn selection(provider: AgentChatProvider) -> AgentChatSelection {
        AgentChatSelection {
            provider,
            model: "model".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        }
    }
}
