use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gent_ports::ConversationActivityLedger;
use gent_protocol::{
    AgentChatIntentFrame,
    model_catalog::{ModelCatalogFrame, ProviderAvailability},
};
use gent_runtime::{
    AgentChatReadService,
    catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile},
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    ConversationActivityFact, PromptHoldReason, ReceiptId,
};

use super::{
    defaults::ModelCatalogDefaults,
    downloads::{
        LocalModelDownloads,
        tests::{GatedTransport, complete_small_model, transfer_started},
    },
    local::LocalModelSource,
    service::ModelCatalogService,
    transport::StandaloneModelCatalog,
};
use crate::{
    CompatibilityAssessment,
    api::RuntimeApi,
    ordinary_lifecycle_cadence::pair_with_standalone_models,
    ordinary_lifecycle_router::OrdinaryPublicLifecycleRouter,
    runtime_facade::{DaemonCompositionState, RuntimeFacade},
    standalone_authority_composition::StandaloneClaurstModels,
    standalone_provider_readiness::AllowStandalonePublicProviders,
};

async fn activity_until(
    ledger: &gent_store::SqliteLedger,
    conversation_id: &str,
    run_id: &str,
    found: impl Fn(&ConversationActivityFact) -> bool,
) -> Vec<ConversationActivityFact> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let facts = ledger
                .read_conversation_activity_page(conversation_id, run_id, 0, 20)
                .unwrap()
                .facts;
            if facts.iter().any(&found) {
                return facts;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected activity fact")
}

#[tokio::test]
async fn a_fresh_install_without_hosted_providers_chats_with_the_default_local_model() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::TurnFollow,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let ledger = state.ledger().clone();
    let host_epoch = state.coordinator().status().unwrap().host_epoch;
    let transport = Arc::new(GatedTransport::default());
    let models = StandaloneClaurstModels::from_data_dir(
        directory.path(),
        crate::local_model_events::LocalModelEvents::new(ledger.clone(), host_epoch),
    )
    .unwrap()
    .with_download_transport(Arc::new(Arc::clone(&transport)));
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), vec![])
            .unwrap(),
    ));
    let (control, ingress, cadence) = pair_with_standalone_models(
        router,
        ledger.clone(),
        host_epoch,
        models.clone(),
        Arc::new(AllowStandalonePublicProviders),
    );
    let defaults = ModelCatalogDefaults::open(directory.path());
    let downloads = LocalModelDownloads::new(models.clone(), defaults.clone());
    let service = ModelCatalogService::new(
        vec![Arc::new(LocalModelSource {
            models: models.clone(),
        })],
        defaults,
    );
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        models.clone(),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap()
    .with_model_catalog(Arc::new(StandaloneModelCatalog {
        service,
        downloads: downloads.clone(),
        initialize: no_claude(),
    }));
    let _cadence = tokio::spawn(cadence.run());
    control.wait_until_ready().await.unwrap();

    assert!(downloads.start_default_when_missing().unwrap());
    let port = runtime.model_catalog_port().unwrap();
    let ModelCatalogFrame::ModelCatalog { catalog, .. } = port
        .exchange(ModelCatalogFrame::ReadModelCatalog {
            request_id: "catalog".into(),
            refresh: true,
        })
        .unwrap()
    else {
        panic!("catalog reply");
    };
    assert_eq!(
        catalog.default_selection.provider,
        AgentChatProvider::Claurst
    );
    assert_eq!(catalog.default_selection.model, "qwen3-1-7b-q4-k-m");

    let created = runtime
        .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("create".into()),
            receipt_id: ReceiptId("create-receipt".into()),
            workspace_path: workspace.path().display().to_string(),
            selection: None,
        })
        .unwrap();
    let [
        AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("a conversation without a selection must be created");
    };
    let selection = AgentChatReadService::new(ledger.clone())
        .run_selection(&conversation_id.0, &run_id.0)
        .unwrap();
    assert_eq!(selection.provider, AgentChatProvider::Claurst);
    assert_eq!(selection.model, "qwen3-1-7b-q4-k-m");
    assert_eq!(selection.effort, AgentChatEffort::Medium);
    assert_eq!(selection.mode, AgentChatMode::Agent);

    runtime
        .agent_chat_intent(AgentChatIntentFrame::SendPrompt {
            request_id: AgentChatRequestId("prompt".into()),
            receipt_id: ReceiptId("prompt-receipt".into()),
            conversation_id: AgentChatConversationId(conversation_id.0.clone()),
            text: "hello".into(),
            attachment_ids: Vec::new(),
        })
        .unwrap();
    activity_until(&ledger, &conversation_id.0, &run_id.0, |fact| {
        matches!(
            fact,
            ConversationActivityFact::PromptHeld {
                reason: PromptHoldReason::ModelDownload,
                ..
            }
        )
    })
    .await;

    transfer_started(&transport).await;
    complete_small_model(&models);
    transport.finish.notify_one();
    activity_until(&ledger, &conversation_id.0, &run_id.0, |fact| {
        matches!(fact, ConversationActivityFact::PromptReleased { .. })
    })
    .await;
    assert_eq!(
        transport.requests.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(matches!(
        port.exchange(ModelCatalogFrame::ReadModelCatalog {
            request_id: "catalog".into(),
            refresh: false,
        }),
        Ok(ModelCatalogFrame::ModelCatalog { catalog, .. })
            if catalog.providers[0].availability == ProviderAvailability::Ready
    ));
}

#[tokio::test]
async fn the_most_recent_in_conversation_switch_becomes_the_next_new_conversation_default() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let profile = RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::LocalModels,
    ]);
    let state = DaemonCompositionState::open(
        directory.path(),
        &profile,
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let ledger = state.ledger().clone();
    let host_epoch = state.coordinator().status().unwrap().host_epoch;
    let models = crate::local_model_jobs::tests::models(directory.path());
    let router = Arc::new(Mutex::new(
        OrdinaryPublicLifecycleRouter::new(AgentChatReadService::new(ledger.clone()), vec![])
            .unwrap(),
    ));
    let (_, ingress, _) = pair_with_standalone_models(
        router,
        ledger.clone(),
        host_epoch,
        models.clone(),
        Arc::new(AllowStandalonePublicProviders),
    );
    let defaults = ModelCatalogDefaults::open(directory.path());
    let service = ModelCatalogService::new(
        vec![Arc::new(LocalModelSource {
            models: models.clone(),
        })],
        defaults.clone(),
    );
    let runtime = RuntimeFacade::from_state_with_standalone_authority(
        state,
        None,
        ingress,
        models.clone(),
        None,
        None,
        None,
        0,
        Vec::new(),
        None,
    )
    .unwrap()
    .with_model_catalog(Arc::new(StandaloneModelCatalog {
        service,
        downloads: LocalModelDownloads::new(models, defaults),
        initialize: no_claude(),
    }));
    let create = |request: &str| {
        let [
            AgentChatIntentFrame::Created {
                conversation_id,
                run_id,
                ..
            },
        ] = &runtime
            .agent_chat_intent(AgentChatIntentFrame::CreateConversation {
                request_id: AgentChatRequestId(request.into()),
                receipt_id: ReceiptId(format!("{request}-receipt")),
                workspace_path: workspace.path().display().to_string(),
                selection: None,
            })
            .unwrap()[..]
        else {
            panic!("created");
        };
        (conversation_id.clone(), run_id.clone())
    };
    let (conversation_id, run_id) = create("first");
    runtime
        .agent_chat_intent(AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("switch".into()),
            receipt_id: ReceiptId("switch-receipt".into()),
            conversation_id,
            parent_run_id: run_id,
            selection: gent_types::AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-8b-q4-k-m".into(),
                effort: AgentChatEffort::High,
                mode: AgentChatMode::Ask,
            },
            context_policy: gent_types::ContextPolicy::Preserve,
        })
        .unwrap();

    let (next_conversation, next_run) = create("second");
    let next = AgentChatReadService::new(ledger)
        .run_selection(&next_conversation.0, &next_run.0)
        .unwrap();
    assert_eq!(next.provider, AgentChatProvider::Claurst);
    assert_eq!(next.model, "qwen3-8b-q4-k-m");
    assert_eq!(
        ModelCatalogDefaults::open(directory.path())
            .selection()
            .model,
        "qwen3-8b-q4-k-m"
    );
}

fn no_claude() -> Arc<super::claude_initialize::ClaudeInitialize> {
    super::claude_initialize::ClaudeInitialize::new(
        crate::provider_executables::ProviderExecutables::explicit(None, None),
        std::time::Duration::from_secs(1),
    )
}
