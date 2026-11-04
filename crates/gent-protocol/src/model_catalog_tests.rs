use gent_types::{AgentChatEffort, AgentChatProvider};
use serde_json::json;

use super::{
    CatalogModel, LOCAL_MODEL_FILES_CAPABILITY, LocalModelAvailability, LocalModelFileRejection,
    ModelCatalog, ModelCatalogFrame, ModelCatalogFrameError, ModelCatalogSelection, ModelListing,
    ProviderAvailability, ProviderModelCatalog,
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
                        removable: false,
                        file_name: None,
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
                    "local":{"sizeBytes":20,"install":{"state":"downloading","downloaded_bytes":5,"total_bytes":20},
                        "removable":false,"fileName":null}}]}]}}})
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

fn user_catalog(install: LocalModelInstallState, file_name: Option<&str>) -> ModelCatalogFrame {
    let mut frame = local_catalog(install);
    if let ModelCatalogFrame::ModelCatalog { catalog, .. } = &mut frame {
        let local = catalog.providers[0].models[0].local.as_mut().unwrap();
        local.removable = file_name.is_some();
        local.file_name = file_name.map(str::to_owned);
    }
    frame
}

#[test]
fn local_file_frames_round_trip_with_typed_rejection_codes() {
    assert_eq!(LOCAL_MODEL_FILES_CAPABILITY, "local-model-files-v1");
    let add =
        json!({"type":"addLocalModel","body":{"requestId":"add-1","path":"C:\\models\\a.gguf"}});
    let frame = serde_json::from_value::<ModelCatalogFrame>(add.clone()).unwrap();
    frame.validate().unwrap();
    assert_eq!(serde_json::to_value(&frame).unwrap(), add);
    let remove = json!({"type":"removeLocalModel","body":{"requestId":"remove-1","modelId":"user-a-1234abcd"}});
    let frame = serde_json::from_value::<ModelCatalogFrame>(remove.clone()).unwrap();
    frame.validate().unwrap();
    assert_eq!(serde_json::to_value(&frame).unwrap(), remove);
    assert!(
        serde_json::from_value::<ModelCatalogFrame>(json!({
            "type":"addLocalModel","body":{"requestId":"a","path":"/m.gguf","args":"--bad"}
        }))
        .is_err()
    );
    for (rejection, code) in [
        (LocalModelFileRejection::NotGguf, "localModelNotGguf"),
        (
            LocalModelFileRejection::MissingChatTemplate,
            "localModelMissingChatTemplate",
        ),
        (
            LocalModelFileRejection::ContextTooSmall,
            "localModelContextTooSmall",
        ),
        (
            LocalModelFileRejection::UnsupportedPath,
            "localModelUnsupportedPath",
        ),
        (LocalModelFileRejection::Unreadable, "localModelUnreadable"),
        (
            LocalModelFileRejection::NotRemovable,
            "localModelNotRemovable",
        ),
    ] {
        assert_eq!(rejection.code(), code);
        let wire = serde_json::to_value(rejection).unwrap();
        assert_eq!(
            serde_json::from_value::<LocalModelFileRejection>(wire).unwrap(),
            rejection
        );
    }
}

#[test]
fn rejects_unsafe_local_file_requests() {
    for path in [
        String::new(),
        " /m.gguf".into(),
        "/m.gguf\n".into(),
        "a".repeat(4_097),
    ] {
        assert_eq!(
            ModelCatalogFrame::AddLocalModel {
                request_id: "add-1".into(),
                path,
            }
            .validate(),
            Err(ModelCatalogFrameError::InvalidPath)
        );
    }
    assert_eq!(
        ModelCatalogFrame::RemoveLocalModel {
            request_id: "remove-1".into(),
            model_id: String::new(),
        }
        .validate(),
        Err(ModelCatalogFrameError::InvalidModelId)
    );
}

#[test]
fn user_files_are_removable_by_file_name_and_may_be_missing() {
    let missing = user_catalog(LocalModelInstallState::Missing, Some("a.gguf"));
    missing.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&missing).unwrap()["body"]["catalog"]["providers"][0]["models"][0]["local"],
        json!({"sizeBytes":20,"install":{"state":"missing"},"removable":true,"fileName":"a.gguf"})
    );
    let mut unpaired = user_catalog(LocalModelInstallState::NotInstalled, None);
    if let ModelCatalogFrame::ModelCatalog { catalog, .. } = &mut unpaired {
        catalog.providers[0].models[0]
            .local
            .as_mut()
            .unwrap()
            .removable = true;
    }
    for invalid in [
        user_catalog(LocalModelInstallState::Missing, None),
        user_catalog(
            LocalModelInstallState::Ready { size_bytes: 20 },
            Some("dir/a.gguf"),
        ),
        user_catalog(
            LocalModelInstallState::Ready { size_bytes: 20 },
            Some("C:\\a.gguf"),
        ),
        unpaired,
    ] {
        assert_eq!(
            invalid.validate(),
            Err(ModelCatalogFrameError::InvalidModel)
        );
    }
}
