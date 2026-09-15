use std::sync::Arc;

use gent_protocol::{
    AgentChatIntentFrame, AgentChatTranscriptFrame, HistoricalTranscriptEntry,
    agent_chat_commands::{AgentChatCommandFrame, CommandListing, CommandOutcome},
    model_catalog::{
        CatalogModel, ModelCatalog, ModelCatalogFrame, ModelCatalogSelection, ModelListing,
        ProviderAvailability, ProviderModelCatalog,
    },
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{
    AgentChatCommandDispatch, AgentChatCommandIntent, AgentChatCommandOrigin,
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, NormalizedTranscriptKind, ReceiptId,
};
use serde_json::json;

use crate::{
    CompatibilityAssessment, RuntimeFacade, agent_chat_intent_error::AgentChatIntentError,
    api::RuntimeApi, build_runtime, runtime_facade::model_catalog::commands::ProviderCommands,
};

struct Catalog;

fn model(id: &str, efforts: Vec<AgentChatEffort>) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: id.into(),
        description: None,
        is_default: id != "gpt-5.6-luna",
        default_effort: efforts.first().copied(),
        efforts,
        local: None,
    }
}

impl crate::runtime_facade::ModelCatalogPort for Catalog {
    fn exchange(&self, frame: ModelCatalogFrame) -> Result<ModelCatalogFrame, String> {
        let ModelCatalogFrame::ReadModelCatalog { request_id, .. } = frame else {
            return Err("unexpected".into());
        };
        let entry = |provider, models| ProviderModelCatalog {
            provider,
            label: format!("{provider:?}"),
            availability: ProviderAvailability::Ready,
            listing: ModelListing::Ready,
            models,
        };
        Ok(ModelCatalogFrame::ModelCatalog {
            request_id,
            catalog: ModelCatalog {
                default_selection: ModelCatalogSelection {
                    provider: AgentChatProvider::Claude,
                    model: "default".into(),
                },
                providers: vec![
                    entry(
                        AgentChatProvider::Claude,
                        vec![model("default", vec![AgentChatEffort::Medium])],
                    ),
                    entry(
                        AgentChatProvider::Codex,
                        vec![
                            model("gpt-5.6", vec![AgentChatEffort::Low, AgentChatEffort::High]),
                            model("gpt-5.6-luna", vec![AgentChatEffort::Medium]),
                        ],
                    ),
                ],
            },
        })
    }
    fn default_selection(&self) -> AgentChatSelection {
        selection(AgentChatProvider::Claude, "default")
    }
    fn remember(&self, _: &AgentChatSelection) {}
    fn validate(&self, _: &AgentChatSelection) -> Result<(), AgentChatIntentError> {
        Ok(())
    }
    fn provider_commands(
        &self,
        provider: AgentChatProvider,
        _: Option<std::path::PathBuf>,
        _: bool,
    ) -> ProviderCommands {
        if provider != AgentChatProvider::Claude {
            return ProviderCommands::none();
        }
        let initialize = json!({"commands": [
            {"name": "context", "description": "Show context usage"},
            {"name": "compact", "description": "Compact the conversation", "argumentHint": "<optional custom summarization instructions>"},
            {"name": "mcp", "description": "Manage MCP servers"},
            {"name": "clear", "description": "Clear", "aliases": ["new", "reset"]},
            {"name": "my-skill", "description": "A project skill"}
        ]});
        ProviderCommands {
            listing: CommandListing::Ready,
            commands: gent_drivers::claude_commands::claude_command_descriptors(&initialize)
                .unwrap(),
        }
    }
}

fn selection(provider: AgentChatProvider, model: &str) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Agent,
    }
}

fn runtime(directory: &std::path::Path) -> RuntimeFacade {
    build_runtime(
        directory,
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]),
        CompatibilityAssessment::default(),
    )
    .unwrap()
    .with_model_catalog(Arc::new(Catalog))
}

fn create(
    runtime: &RuntimeFacade,
    id: &str,
    provider: AgentChatProvider,
    model: &str,
) -> AgentChatConversationId {
    match runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId(format!("create-{id}")),
            receipt_id: ReceiptId(format!("create-receipt-{id}")),
            workspace_path: ".".into(),
            selection: Some(selection(provider, model)),
        })
        .unwrap()
        .as_slice()
    {
        [
            AgentChatIntentFrame::Created {
                conversation_id, ..
            },
        ] => conversation_id.clone(),
        other => panic!("{other:?}"),
    }
}

fn invoke(
    runtime: &RuntimeFacade,
    receipt: &str,
    conversation_id: Option<&AgentChatConversationId>,
    name: &str,
    arguments: &str,
) -> Result<CommandOutcome, AgentChatIntentError> {
    match runtime.agent_chat_command(AgentChatCommandFrame::InvokeCommand {
        request_id: AgentChatRequestId(format!("request-{receipt}")),
        receipt_id: ReceiptId(receipt.into()),
        conversation_id: conversation_id.cloned(),
        workspace_path: Some(".".into()),
        name: name.into(),
        arguments: arguments.into(),
    })? {
        AgentChatCommandFrame::CommandInvoked { outcome, .. } => Ok(outcome),
        other => panic!("{other:?}"),
    }
}

fn catalog(
    runtime: &RuntimeFacade,
    conversation_id: &AgentChatConversationId,
) -> gent_protocol::agent_chat_commands::CommandCatalog {
    match runtime
        .agent_chat_command(AgentChatCommandFrame::ReadCommandCatalog {
            request_id: AgentChatRequestId("catalog".into()),
            conversation_id: Some(conversation_id.clone()),
            workspace_path: None,
            refresh: false,
        })
        .unwrap()
    {
        AgentChatCommandFrame::CommandCatalog { catalog, .. } => catalog,
        other => panic!("{other:?}"),
    }
}

fn transcript(
    runtime: &RuntimeFacade,
    conversation_id: &AgentChatConversationId,
) -> Vec<(NormalizedTranscriptKind, String)> {
    match runtime
        .agent_chat_transcript(AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.0.clone(),
            after_cursor: None,
            limit: 50,
        })
        .unwrap()
    {
        AgentChatTranscriptFrame::Page(page) => page
            .events
            .into_iter()
            .map(|event| (event.kind, event.text))
            .collect(),
        other @ AgentChatTranscriptFrame::PageRequest { .. } => panic!("{other:?}"),
    }
}

fn run_selection(
    runtime: &RuntimeFacade,
    conversation_id: &AgentChatConversationId,
) -> AgentChatSelection {
    runtime
        .agent_chat_reads
        .as_ref()
        .unwrap()
        .detail(&conversation_id.0)
        .unwrap()
        .summary
        .selection
}

#[path = "runtime_facade_commands_intent_tests.rs"]
mod intent_tests;

#[test]
fn claude_catalog_classifies_builtins_skills_and_reserved_gent_names_while_codex_has_only_gent() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let claude = catalog(
        &runtime,
        &create(&runtime, "claude", AgentChatProvider::Claude, "default"),
    );
    let find = |name: &str| {
        claude
            .commands
            .iter()
            .find(|command| command.answers_to(name))
            .cloned()
            .unwrap()
    };
    let provider = AgentChatProvider::Claude;
    assert_eq!(claude.scope.provider, Some(provider));
    assert_eq!(
        find("context").origin,
        AgentChatCommandOrigin::ProviderBuiltin { provider }
    );
    assert_eq!(
        find("context").dispatch,
        AgentChatCommandDispatch::ProviderNative
    );
    assert!(
        matches!(find("mcp").dispatch, AgentChatCommandDispatch::Unsupported { use_instead: Some(ref value), .. } if value == "/tools")
    );
    assert_eq!(
        find("my-skill").origin,
        AgentChatCommandOrigin::ProviderSkill { provider }
    );
    assert_eq!(find("reset").origin, AgentChatCommandOrigin::Gent);
    assert_eq!(
        find("new").dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::CreateConversation
        }
    );
    assert!(claude.revision.starts_with("sha256:"));
    let codex = catalog(
        &runtime,
        &create(&runtime, "codex", AgentChatProvider::Codex, "gpt-5.6"),
    );
    assert!(
        codex
            .commands
            .iter()
            .all(|command| command.origin == AgentChatCommandOrigin::Gent)
    );
    assert_eq!(
        codex
            .commands
            .iter()
            .find(|command| command.name == "compact")
            .unwrap()
            .dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::Compact
        }
    );
    assert_eq!(
        find("compact").dispatch,
        AgentChatCommandDispatch::ProviderNative
    );
    assert_eq!(
        find("compact").origin,
        AgentChatCommandOrigin::ProviderBuiltin { provider }
    );
    assert_ne!(codex.revision, claude.revision);
}

#[test]
fn a_slash_prompt_is_rejected_before_any_turn_exists_but_a_path_is_a_prompt() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "prompt", AgentChatProvider::Claude, "default");
    let send = |receipt: &str, text: &str, queued: bool| {
        let (request_id, receipt_id) = (
            AgentChatRequestId(receipt.into()),
            ReceiptId(receipt.into()),
        );
        let (conversation_id, text, attachment_ids) =
            (conversation_id.clone(), text.to_owned(), Vec::new());
        runtime.agent_chat_intent(if queued {
            AgentChatIntentFrame::QueuePrompt {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
            }
        } else {
            AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
            }
        })
    };
    for (receipt, text, queued) in [
        ("nope", "/nope", false),
        ("context", "  /context now", true),
    ] {
        let error = send(receipt, text, queued).unwrap_err();
        assert_eq!(error.code, "slashCommandRequiresInvoke");
    }
    assert!(transcript(&runtime, &conversation_id).is_empty());
    assert!(send("path", "/Users/me/notes.md summarize", false).is_ok());
    assert_eq!(transcript(&runtime, &conversation_id).len(), 1);
}

#[test]
fn unknown_unsupported_client_and_scoped_commands_are_typed_rejections() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "reject", AgentChatProvider::Claude, "default");
    let code = |receipt: &str, conversation: Option<&AgentChatConversationId>, name: &str| {
        invoke(&runtime, receipt, conversation, name, "")
            .unwrap_err()
            .code
    };
    assert_eq!(
        code("unknown", Some(&conversation_id), "nope"),
        "unknownCommand"
    );
    assert_eq!(
        code("mcp", Some(&conversation_id), "mcp"),
        "unsupportedCommand"
    );
    let local = create(
        &runtime,
        "reject-local",
        AgentChatProvider::Claurst,
        "qwen3-1-7b-q4-k-m",
    );
    assert_eq!(
        code("compact-local", Some(&local), "compact"),
        "commandRequiresProviderSession"
    );
    let codex = create(
        &runtime,
        "reject-codex",
        AgentChatProvider::Codex,
        "gpt-5.6",
    );
    assert_eq!(
        code("compact-codex", Some(&codex), "compact"),
        "commandRequiresProviderSession"
    );
    assert_eq!(
        invoke(
            &runtime,
            "compact-codex-args",
            Some(&codex),
            "compact",
            "keep tests"
        )
        .unwrap_err()
        .code,
        "commandArgumentsInvalid"
    );

    assert_eq!(
        code("resume", Some(&conversation_id), "resume"),
        "clientActionCommand"
    );
    assert_eq!(
        code("scoped", None, "effort"),
        "commandRequiresConversation"
    );
    assert_eq!(
        invoke(
            &runtime,
            "arguments",
            Some(&conversation_id),
            "effort",
            "loud"
        )
        .unwrap_err()
        .code,
        "commandArgumentsInvalid"
    );
    assert!(transcript(&runtime, &conversation_id).is_empty());
}

#[test]
fn a_provider_native_command_is_delivered_verbatim_once_per_receipt_and_blocks_while_its_turn_runs()
{
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "native", AgentChatProvider::Claude, "default");
    let first = invoke(&runtime, "native", Some(&conversation_id), "context", "").unwrap();
    let replay = invoke(&runtime, "native", Some(&conversation_id), "context", "").unwrap();
    assert!(matches!(first, CommandOutcome::Delivered { .. }));
    assert_eq!(first, replay);
    assert_eq!(
        transcript(&runtime, &conversation_id),
        [(NormalizedTranscriptKind::UserMessage, "/context".to_owned())]
    );
    assert_eq!(
        invoke(
            &runtime,
            "native-busy",
            Some(&conversation_id),
            "my-skill",
            "go"
        )
        .unwrap_err()
        .code,
        "commandBlockedByActiveTurn"
    );
    assert!(
        invoke(
            &runtime,
            "native",
            Some(&conversation_id),
            "context",
            "other"
        )
        .is_err()
    );
}

#[test]
fn a_provider_native_command_waits_for_a_provider_session_when_history_exists() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(&runtime, "history", AgentChatProvider::Claude, "default");
    let detail = runtime
        .agent_chat_reads
        .as_ref()
        .unwrap()
        .detail(&conversation_id.0)
        .unwrap();
    runtime
        .agent_chat_intent(AgentChatIntentFrame::ImportTranscript {
            request_id: AgentChatRequestId("import".into()),
            conversation_id: conversation_id.clone(),
            run_id: gent_types::AgentChatRunId(detail.current_run_id),
            entries: vec![HistoricalTranscriptEntry {
                source_id: "user-1".into(),
                kind: NormalizedTranscriptKind::UserMessage,
                text: "Explain the repository.".into(),
            }],
        })
        .unwrap();
    assert_eq!(
        invoke(&runtime, "history", Some(&conversation_id), "context", "")
            .unwrap_err()
            .code,
        "commandRequiresProviderSession"
    );
}

#[test]
fn a_local_model_compact_is_a_receipt_backed_gent_turn_once_history_exists() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = runtime(directory.path());
    let conversation_id = create(
        &runtime,
        "local-compact",
        AgentChatProvider::Claurst,
        "qwen3-1-7b-q4-k-m",
    );
    let detail = runtime
        .agent_chat_reads
        .as_ref()
        .unwrap()
        .detail(&conversation_id.0)
        .unwrap();
    runtime
        .agent_chat_intent(AgentChatIntentFrame::ImportTranscript {
            request_id: AgentChatRequestId("import-local".into()),
            conversation_id: conversation_id.clone(),
            run_id: gent_types::AgentChatRunId(detail.current_run_id),
            entries: vec![HistoricalTranscriptEntry {
                source_id: "user-1".into(),
                kind: NormalizedTranscriptKind::UserMessage,
                text: "Remember LARK-7.".into(),
            }],
        })
        .unwrap();
    let compact = catalog(&runtime, &conversation_id)
        .commands
        .into_iter()
        .find(|command| command.name == "compact")
        .unwrap();
    assert_eq!(
        compact.dispatch,
        AgentChatCommandDispatch::GentIntent {
            intent: AgentChatCommandIntent::Compact
        }
    );
    assert_eq!(
        invoke(
            &runtime,
            "local-compact-args",
            Some(&conversation_id),
            "compact",
            "keep codes"
        )
        .unwrap_err()
        .code,
        "commandArgumentsInvalid"
    );
    let first = invoke(
        &runtime,
        "local-compact",
        Some(&conversation_id),
        "compact",
        "",
    )
    .unwrap();
    let replay = invoke(
        &runtime,
        "local-compact",
        Some(&conversation_id),
        "compact",
        "",
    )
    .unwrap();
    assert!(matches!(first, CommandOutcome::Delivered { .. }));
    assert_eq!(first, replay);
    assert_eq!(
        transcript(&runtime, &conversation_id)
            .into_iter()
            .filter(|(_, text)| text == "/compact")
            .count(),
        1
    );
    assert_eq!(
        invoke(
            &runtime,
            "local-compact-busy",
            Some(&conversation_id),
            "compact",
            ""
        )
        .unwrap_err()
        .code,
        "commandBlockedByActiveTurn"
    );
}
