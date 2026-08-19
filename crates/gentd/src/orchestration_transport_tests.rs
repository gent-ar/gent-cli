//! Observer-safety tests for the reserved task-graph orchestration transport.

use gent_protocol::{ORCHESTRATION_CAPABILITY, OrchestrationFrame};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::AgentChatConversationId;

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_orchestration_authority() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(
        !runtime
            .capabilities()
            .unwrap()
            .0
            .iter()
            .any(|capability| capability == ORCHESTRATION_CAPABILITY)
    );
    let error = runtime
        .orchestration(OrchestrationFrame::GraphRead {
            request_id: "request-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            graph_id: "graph-1".into(),
        })
        .unwrap_err();
    assert_eq!(
        error,
        "orchestration is unavailable while gentd is observer-disabled"
    );
}

#[test]
fn approved_chat_persistence_profile_advertises_and_reads_orchestration_graphs() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(
        runtime
            .capabilities()
            .unwrap()
            .0
            .iter()
            .any(|capability| capability == ORCHESTRATION_CAPABILITY)
    );
    assert!(matches!(
        runtime
            .orchestration(OrchestrationFrame::GraphRead {
                request_id: "request-1".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                graph_id: "graph-1".into()
            })
            .unwrap(),
        OrchestrationFrame::Graph { graph: None, .. }
    ));
}
