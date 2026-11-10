use std::{path::Path, sync::Arc};

use gent_protocol::{
    LocalModelInstallState, WireFrame,
    model_catalog::{
        CatalogModel, LOCAL_MODEL_FILES_CAPABILITY, MODEL_CATALOG_CAPABILITY, ModelCatalog,
        ModelCatalogFrame, ModelCatalogSelection,
    },
    read_frame, read_json_frame,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, CapabilitySet};
use serde_json::json;
use tokio::io::duplex;

use super::{ModelCatalogPort, StandaloneModelCatalog, dispatch_port};
use crate::{
    local_model_provisioning::gguf::tests::{Gguf, chat_model},
    runtime_facade::model_catalog::{
        claude_initialize::ClaudeInitialize, defaults::ModelCatalogDefaults,
        downloads::LocalModelDownloads, local::LocalModelSource, service::ModelCatalogService,
    },
};

pub(crate) fn catalog_port(data_dir: &Path) -> StandaloneModelCatalog {
    let models = crate::local_model_jobs::tests::models(data_dir);
    let defaults = ModelCatalogDefaults::open(data_dir);
    StandaloneModelCatalog {
        service: ModelCatalogService::new(
            vec![Arc::new(LocalModelSource {
                models: models.clone(),
            })],
            defaults.clone(),
        ),
        downloads: LocalModelDownloads::new(models, defaults),
        initialize: ClaudeInitialize::new(
            crate::provider_executables::ProviderExecutables::explicit(None, None),
            std::time::Duration::from_secs(1),
        ),
    }
}

fn catalog(reply: ModelCatalogFrame) -> ModelCatalog {
    let ModelCatalogFrame::ModelCatalog { catalog, .. } = reply else {
        panic!("expected a catalog");
    };
    catalog
}

fn user_models(catalog: &ModelCatalog) -> Vec<CatalogModel> {
    catalog.providers[0]
        .models
        .iter()
        .filter(|model| model.local.as_ref().is_some_and(|local| local.removable))
        .cloned()
        .collect()
}

fn add(port: &StandaloneModelCatalog, path: &Path) -> Result<ModelCatalog, &'static str> {
    port.exchange(ModelCatalogFrame::AddLocalModel {
        request_id: "add".into(),
        path: path.to_str().unwrap().into(),
    })
    .map(catalog)
    .map_err(|rejection| rejection.code)
}

fn selection(model: &str) -> gent_types::AgentChatSelection {
    gent_types::AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: model.into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    }
}

#[test]
fn an_added_file_is_listed_ready_removable_and_selectable_in_place() {
    let data = tempfile::tempdir().unwrap();
    let picked = tempfile::tempdir().unwrap();
    let path = picked.path().join("Tiny.gguf");
    std::fs::write(&path, chat_model("Tiny Llama", 8_192).bytes()).unwrap();
    let port = catalog_port(data.path());
    let listed = user_models(&add(&port, &path).unwrap());
    let [model] = listed.as_slice() else {
        panic!("one user model: {listed:?}");
    };
    assert!(model.id.starts_with("user-tiny-llama-"));
    assert_eq!(model.label, "Tiny Llama");
    let local = model.local.as_ref().unwrap();
    assert_eq!(local.file_name.as_deref(), Some("Tiny.gguf"));
    assert_eq!(
        local.install,
        LocalModelInstallState::Ready {
            size_bytes: std::fs::metadata(&path).unwrap().len()
        }
    );
    port.fit(&selection(&model.id)).unwrap();
    assert!(!data.path().join("models").exists());
    assert!(data.path().join("user-models.json").is_file());
    assert_eq!(
        user_models(&add(&port, &path).unwrap()).len(),
        1,
        "re-adding the same file replaces its entry"
    );

    std::fs::remove_file(&path).unwrap();
    let rejected = port.fit(&selection(&model.id)).unwrap_err();
    assert_eq!(rejected.code, "selectionModelUnavailable");
    let missing = user_models(&port.service.snapshot());
    assert_eq!(missing.len(), 1);
    assert_eq!(
        missing[0].local.as_ref().unwrap().install,
        LocalModelInstallState::Missing
    );
    assert!(data.path().join("user-models.json").is_file());
}

#[test]
fn a_changed_file_is_missing_until_it_is_added_again() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("model.gguf");
    std::fs::write(&path, chat_model("Changing", 8_192).bytes()).unwrap();
    let port = catalog_port(data.path());
    let id = user_models(&add(&port, &path).unwrap())[0].id.clone();
    let mut grown = chat_model("Changing", 8_192).bytes();
    grown.extend([0; 16]);
    std::fs::write(&path, grown).unwrap();
    assert_eq!(
        port.fit(&selection(&id)).unwrap_err().code,
        "selectionModelUnavailable"
    );
    let ready = user_models(&add(&port, &path).unwrap());
    assert_eq!(ready.len(), 1);
    assert!(matches!(
        ready[0].local.as_ref().unwrap().install,
        LocalModelInstallState::Ready { .. }
    ));
}

#[test]
fn rejections_and_removal_answer_with_typed_codes() {
    let data = tempfile::tempdir().unwrap();
    let port = catalog_port(data.path());
    let plain = data.path().join("plain.gguf");
    std::fs::write(
        &plain,
        Gguf::default()
            .text("general.architecture", "llama")
            .number("llama.context_length", 8_192)
            .bytes(),
    )
    .unwrap();
    assert_eq!(add(&port, &plain), Err("localModelMissingChatTemplate"));
    assert!(!data.path().join("user-models.json").exists());
    let remove = |model_id: &str| {
        port.exchange(ModelCatalogFrame::RemoveLocalModel {
            request_id: "remove".into(),
            model_id: model_id.into(),
        })
        .map_err(|rejection| rejection.code)
    };
    assert_eq!(
        remove("qwen3-1-7b-q4-k-m").unwrap_err(),
        "localModelNotRemovable"
    );
    let path = data.path().join("good.gguf");
    std::fs::write(&path, chat_model("Good", 8_192).bytes()).unwrap();
    let id = user_models(&add(&port, &path).unwrap())[0].id.clone();
    port.exchange(ModelCatalogFrame::SetDefaultModel {
        request_id: "default".into(),
        selection: ModelCatalogSelection {
            provider: AgentChatProvider::Claurst,
            model: id.clone(),
        },
    })
    .unwrap();
    let after = catalog(remove(&id).unwrap());
    assert!(user_models(&after).is_empty());
    assert_eq!(after.default_selection, ModelCatalogDefaults::built_in());
    assert!(path.is_file());
    assert_eq!(remove(&id).unwrap_err(), "localModelNotRemovable");
}

#[tokio::test]
async fn only_clients_negotiating_local_files_see_or_manage_user_models() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("model.gguf");
    std::fs::write(&path, chat_model("Visible", 8_192).bytes()).unwrap();
    let port: Arc<dyn ModelCatalogPort> = Arc::new(catalog_port(data.path()));
    let add =
        json!({"type":"addLocalModel","body":{"requestId":"add","path":path.to_str().unwrap()}});
    let catalog_only = CapabilitySet(vec![MODEL_CATALOG_CAPABILITY.into()]);
    let with_files = CapabilitySet(vec![
        MODEL_CATALOG_CAPABILITY.into(),
        LOCAL_MODEL_FILES_CAPABILITY.into(),
    ]);
    let (_, mut writer) = duplex(1 << 16);
    assert!(
        !dispatch_port(&mut writer, Some(Arc::clone(&port)), &catalog_only, &add)
            .await
            .unwrap()
    );
    let (mut reader, mut writer) = duplex(1 << 16);
    assert!(
        dispatch_port(&mut writer, Some(Arc::clone(&port)), &with_files, &add)
            .await
            .unwrap()
    );
    let added: serde_json::Value = read_json_frame(&mut reader).await.unwrap();
    let models = added["body"]["catalog"]["providers"][0]["models"]
        .as_array()
        .unwrap();
    assert!(
        models
            .iter()
            .any(|model| model["local"]["removable"] == true
                && model["local"]["fileName"] == "model.gguf")
    );
    assert!(
        !added.to_string().contains(data.path().to_str().unwrap()),
        "the catalog never carries the file's directory"
    );
    let read = json!({"type":"readModelCatalog","body":{"requestId":"read","refresh":false}});
    let (mut reader, mut writer) = duplex(1 << 16);
    dispatch_port(&mut writer, Some(Arc::clone(&port)), &catalog_only, &read)
        .await
        .unwrap();
    let legacy: serde_json::Value = read_json_frame(&mut reader).await.unwrap();
    assert!(
        legacy["body"]["catalog"]["providers"][0]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|model| model["local"]["removable"] == false)
    );
    let bad = json!({"type":"addLocalModel","body":{"requestId":"bad","path":data.path().join("absent.gguf").to_str().unwrap()}});
    let (mut reader, mut writer) = duplex(1 << 16);
    dispatch_port(&mut writer, Some(port), &with_files, &bad)
        .await
        .unwrap();
    let WireFrame::Error { code, .. } = read_frame(&mut reader).await.unwrap() else {
        panic!("expected a typed rejection");
    };
    assert_eq!(code, "localModelUnreadable");
}
