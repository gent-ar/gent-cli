use std::sync::{Arc, Mutex};

use gent_protocol::{
    WireFrame,
    model_catalog::{
        MODEL_CATALOG_CAPABILITY, ModelCatalog, ModelCatalogFrame, ModelCatalogSelection,
    },
    read_frame, read_json_frame,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, CapabilitySet,
};
use serde_json::json;
use tokio::io::duplex;

use super::{ModelCatalogPort, dispatch_port};

#[derive(Default)]
struct RecordingPort {
    requests: Mutex<Vec<ModelCatalogFrame>>,
}

impl ModelCatalogPort for RecordingPort {
    fn exchange(&self, frame: ModelCatalogFrame) -> Result<ModelCatalogFrame, String> {
        self.requests.lock().unwrap().push(frame);
        Ok(ModelCatalogFrame::ModelCatalog {
            request_id: "catalog-1".into(),
            catalog: ModelCatalog {
                default_selection: ModelCatalogSelection {
                    provider: AgentChatProvider::Claurst,
                    model: "qwen3-1-7b-q4-k-m".into(),
                },
                providers: Vec::new(),
            },
        })
    }

    fn remember(&self, _: &AgentChatSelection) {}

    fn validate(
        &self,
        _: &AgentChatSelection,
    ) -> Result<(), crate::agent_chat_intent_error::AgentChatIntentError> {
        Ok(())
    }

    fn default_selection(&self) -> AgentChatSelection {
        AgentChatSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen3-1-7b-q4-k-m".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        }
    }
}

fn capabilities() -> CapabilitySet {
    CapabilitySet(vec![MODEL_CATALOG_CAPABILITY.into()])
}

fn read_request() -> serde_json::Value {
    json!({"type": "readModelCatalog", "body": {"requestId": "catalog-1", "refresh": true}})
}

#[tokio::test]
async fn the_catalog_is_unreachable_without_its_negotiated_capability() {
    let port = Arc::new(RecordingPort::default());
    let (_, mut writer) = duplex(4096);
    assert!(
        !dispatch_port(
            &mut writer,
            Some(Arc::clone(&port) as Arc<dyn ModelCatalogPort>),
            &CapabilitySet::default(),
            &read_request(),
        )
        .await
        .unwrap()
    );
    assert!(port.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_read_is_answered_with_the_correlated_catalog() {
    let port = Arc::new(RecordingPort::default());
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(
            &mut writer,
            Some(Arc::clone(&port) as Arc<dyn ModelCatalogPort>),
            &capabilities(),
            &read_request(),
        )
        .await
        .unwrap()
    );
    let reply: serde_json::Value = read_json_frame(&mut reader).await.unwrap();
    assert_eq!(reply["type"], "modelCatalog");
    assert_eq!(
        reply["body"]["catalog"]["defaultSelection"]["provider"],
        "claurst"
    );
    assert!(matches!(
        port.requests.lock().unwrap().as_slice(),
        [ModelCatalogFrame::ReadModelCatalog { refresh: true, .. }]
    ));
}

#[tokio::test]
async fn a_runtime_without_a_catalog_answers_with_a_typed_error() {
    let (mut reader, mut writer) = duplex(4096);
    let port: Arc<dyn ModelCatalogPort> = Arc::new(RecordingPort::default());
    let catalog = port
        .exchange(ModelCatalogFrame::ReadModelCatalog {
            request_id: "catalog-1".into(),
            refresh: false,
        })
        .unwrap();
    assert!(
        dispatch_port(
            &mut writer,
            None,
            &capabilities(),
            &serde_json::to_value(&catalog).unwrap(),
        )
        .await
        .unwrap()
    );
    let WireFrame::Error { code, .. } = read_frame(&mut reader).await.unwrap() else {
        panic!("expected an error");
    };
    assert_eq!(code, "modelCatalogUnavailable");
}
