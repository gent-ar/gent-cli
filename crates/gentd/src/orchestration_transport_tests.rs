//! Observer-safety tests for the reserved task-graph orchestration transport.

use gent_protocol::{ORCHESTRATION_CAPABILITY, OrchestrationFrame};
use gent_runtime::catalog::declared_capabilities;
use gent_types::AgentChatConversationId;

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_orchestration_authority() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities(),
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
