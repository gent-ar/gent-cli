use gent_types::{AgentChatEffort, AgentChatProvider};
use serde_json::json;

use super::{
    CatalogModel, LocalModelAvailability, ModelCatalog, ModelCatalogFrame, ModelCatalogFrameError,
    ModelCatalogSelection, ModelListing, ProviderAvailability, ProviderModelCatalog,
};
use crate::LocalModelInstallState;

fn local_catalog(install: LocalModelInstallState) -> ModelCatalogFrame {
    ModelCatalogFrame::ModelCatalog {
        request_id: "catalog-1".into(),
        catalog: ModelCatalog {
            default_selection: ModelCatalogSelection {
                provider: AgentChatProvider::Claurst,
                model: "qwen3-1-7b-q4-k-m".into(),
            },
            providers: vec![ProviderModelCatalog {
                provider: AgentChatProvider::Claurst,
                label: "Gent".into(),
                availability: ProviderAvailability::Ready,
                listing: ModelListing::Ready,
                models: vec![CatalogModel {
                    id: "qwen3-1-7b-q4-k-m".into(),
                    label: "Qwen3 1.7B (Q4_K_M)".into(),
                    description: None,
                    is_default: true,
                    efforts: vec![AgentChatEffort::Medium],
                    default_effort: Some(AgentChatEffort::Medium),
                    local: Some(LocalModelAvailability {
                        size_bytes: 20,
                        install,
                    }),
                }],
            }],
        },
    }
}

#[test]
fn catalog_frames_use_distinct_camel_case_wire_names() {
    let frame = local_catalog(LocalModelInstallState::Downloading {
        downloaded_bytes: 5,
        total_bytes: 20,
    });
    frame.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"type":"modelCatalog","body":{"requestId":"catalog-1","catalog":{
            "defaultSelection":{"provider":"claurst","model":"qwen3-1-7b-q4-k-m"},
            "providers":[{"provider":"claurst","label":"Gent","availability":{"state":"ready"},"listing":{"state":"ready"},
                "models":[{"id":"qwen3-1-7b-q4-k-m","label":"Qwen3 1.7B (Q4_K_M)","description":null,
                    "isDefault":true,"efforts":["medium"],"defaultEffort":"medium",
                    "local":{"sizeBytes":20,"install":{"state":"downloading","downloaded_bytes":5,"total_bytes":20}}}]}]}}})
    );
    assert_eq!(
        serde_json::to_value(ModelCatalogFrame::ReadModelCatalog {
            request_id: "catalog-1".into(),
            refresh: true,
        })
        .unwrap(),
        json!({"type":"readModelCatalog","body":{"requestId":"catalog-1","refresh":true}})
    );
}

#[test]
fn rejects_contradictory_local_progress_and_unknown_default_effort() {
    assert_eq!(
        local_catalog(LocalModelInstallState::Downloading {
            downloaded_bytes: 21,
            total_bytes: 20,
        })
        .validate(),
        Err(ModelCatalogFrameError::InvalidModel)
    );
    let mut frame = local_catalog(LocalModelInstallState::NotInstalled);
    if let ModelCatalogFrame::ModelCatalog { catalog, .. } = &mut frame {
        catalog.providers[0].models[0].default_effort = Some(AgentChatEffort::Ultra);
    }
    assert_eq!(frame.validate(), Err(ModelCatalogFrameError::InvalidModel));
}

#[test]
fn rejects_duplicate_providers_and_unsafe_request_values() {
    let mut frame = local_catalog(LocalModelInstallState::NotInstalled);
    if let ModelCatalogFrame::ModelCatalog { catalog, .. } = &mut frame {
        catalog.providers.push(catalog.providers[0].clone());
    }
    assert_eq!(
        frame.validate(),
        Err(ModelCatalogFrameError::InvalidProvider)
    );
    assert_eq!(
        ModelCatalogFrame::CancelModelDownload {
            request_id: "cancel-1".into(),
            model_id: " ".into(),
        }
        .validate(),
        Err(ModelCatalogFrameError::InvalidModelId)
    );
    assert!(
        serde_json::from_value::<ModelCatalogFrame>(json!({
            "type":"startModelDownload","body":{"requestId":"a","modelId":"m","url":"https://bad.test"}
        }))
        .is_err()
    );
}
