//! Daemon mapping for durable per-conversation advanced launch configuration.

use gent_ports::{AgentChatConversationConfigLedger, AgentChatReadLedger, Ledger};
use gent_protocol::AgentChatConversationConfigFrame;
use gent_runtime::{AgentChatReadService, Coordinator};
use gent_types::{
    AgentChatConversationConfigRecord, AgentChatConversationConfigUnsupportedField,
    AgentChatProvider,
};

/// Handles one local conversation-config exchange without provider or process dependencies.
pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    reads: Option<&AgentChatReadService<L>>,
    frame: AgentChatConversationConfigFrame,
) -> Result<AgentChatConversationConfigFrame, String>
where
    L: Ledger + AgentChatConversationConfigLedger + AgentChatReadLedger,
{
    match frame {
        AgentChatConversationConfigFrame::Current {
            request_id,
            conversation_id,
        } => {
            let config = coordinator
                .current_conversation_config(&conversation_id)
                .map_err(|error| error.to_string())?;
            let unsupported_for_provider =
                unsupported_fields(reads, &conversation_id, config.as_ref());
            Ok(AgentChatConversationConfigFrame::CurrentConfig {
                request_id,
                config,
                unsupported_for_provider,
            })
        }
        AgentChatConversationConfigFrame::Save { request_id, config } => {
            coordinator
                .save_conversation_config(&config)
                .map_err(|error| error.to_string())?;
            let unsupported_for_provider =
                unsupported_fields(reads, &config.conversation_id.0, Some(&config));
            Ok(AgentChatConversationConfigFrame::Saved {
                request_id,
                config,
                unsupported_for_provider,
            })
        }
        AgentChatConversationConfigFrame::CurrentConfig { .. }
        | AgentChatConversationConfigFrame::Saved { .. } => {
            Err("conversation config response frames are server-only".into())
        }
    }
}

fn unsupported_fields<L: AgentChatReadLedger>(
    reads: Option<&AgentChatReadService<L>>,
    conversation_id: &str,
    config: Option<&AgentChatConversationConfigRecord>,
) -> Vec<AgentChatConversationConfigUnsupportedField> {
    let (Some(reads), Some(config)) = (reads, config) else {
        return Vec::new();
    };
    let Ok(summary) = reads.summary(conversation_id) else {
        return Vec::new();
    };
    unsupported_for_provider(summary.selection.provider, config)
}

fn unsupported_for_provider(
    provider: AgentChatProvider,
    config: &AgentChatConversationConfigRecord,
) -> Vec<AgentChatConversationConfigUnsupportedField> {
    match provider {
        AgentChatProvider::Claude => Vec::new(),
        AgentChatProvider::Codex | AgentChatProvider::Claurst => {
            let mut unsupported = Vec::new();
            if config.system_prompt.is_some() && !config.append_system_prompt {
                unsupported.push(AgentChatConversationConfigUnsupportedField::SystemPromptOverride);
            }
            if config.max_turns.is_some() {
                unsupported.push(AgentChatConversationConfigUnsupportedField::MaxTurns);
            }
            if !config.disallowed_tools.is_empty() {
                unsupported.push(AgentChatConversationConfigUnsupportedField::DisallowedTools);
            }
            unsupported
        }
    }
}

#[cfg(test)]
mod tests {
    use super::exchange;
    use gent_ports::AgentChatWorkspaceLedger;
    use gent_protocol::AgentChatConversationConfigFrame;
    use gent_runtime::{AgentChatReadService, Coordinator};
    use gent_store::SqliteLedger;
    use gent_types::{
        AgentChatConversationConfigRecord, AgentChatConversationCreate, AgentChatConversationId,
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId, AgentChatSelection,
        CapabilitySet, HostEpoch, ReceiptId, WorkspaceRecord,
    };

    fn config(revision: u64) -> AgentChatConversationConfigRecord {
        AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation-1".into()),
            revision,
            system_prompt: Some("Be concise.".into()),
            append_system_prompt: true,
            max_turns: Some(5),
            disallowed_tools: vec!["shell:rm".into()],
        }
    }

    fn conversation(ledger: &SqliteLedger, conversation_id: &str) {
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId(format!("{conversation_id}-create-receipt")),
                    idempotency_key: format!("{conversation_id}-create-key"),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId(conversation_id.into()),
                    run_id: AgentChatRunId(format!("{conversation_id}-run")),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-sonnet".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: "/workspace-1".into(),
                },
            )
            .unwrap();
    }

    #[test]
    fn save_then_current_round_trips_without_a_reads_service() {
        let ledger = SqliteLedger::in_memory().unwrap();
        conversation(&ledger, "conversation-1");
        let coordinator = Coordinator::new(ledger, CapabilitySet::default());
        let saved = exchange(
            &coordinator,
            None,
            AgentChatConversationConfigFrame::Save {
                request_id: "request-1".into(),
                config: config(1),
            },
        )
        .unwrap();
        assert!(matches!(
            saved,
            AgentChatConversationConfigFrame::Saved { unsupported_for_provider, .. }
            if unsupported_for_provider.is_empty()
        ));
        let current = exchange(
            &coordinator,
            None,
            AgentChatConversationConfigFrame::Current {
                request_id: "request-2".into(),
                conversation_id: "conversation-1".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            current,
            AgentChatConversationConfigFrame::CurrentConfig { config: Some(config), .. }
            if config.revision == 1
        ));
    }

    #[test]
    fn stale_revision_is_rejected() {
        let ledger = SqliteLedger::in_memory().unwrap();
        conversation(&ledger, "conversation-1");
        let coordinator = Coordinator::new(ledger, CapabilitySet::default());
        exchange(
            &coordinator,
            None,
            AgentChatConversationConfigFrame::Save {
                request_id: "request-1".into(),
                config: config(1),
            },
        )
        .unwrap();
        let rejected = exchange(
            &coordinator,
            None,
            AgentChatConversationConfigFrame::Save {
                request_id: "request-2".into(),
                config: config(1),
            },
        );
        assert!(rejected.is_err());
    }

    #[test]
    fn a_codex_conversation_reports_unsupported_fields() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation-receipt".into()),
                    idempotency_key: "conversation-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Codex,
                        model: "gpt-5.6".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: "/workspace".into(),
                },
            )
            .unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        let reads = AgentChatReadService::new(ledger);
        let saved = exchange(
            &coordinator,
            Some(&reads),
            AgentChatConversationConfigFrame::Save {
                request_id: "request-1".into(),
                config: config(1),
            },
        )
        .unwrap();
        let AgentChatConversationConfigFrame::Saved {
            unsupported_for_provider,
            ..
        } = saved
        else {
            unreachable!()
        };
        assert_eq!(unsupported_for_provider.len(), 2);
    }
}
