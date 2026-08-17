//! Observer-safety tests for the reserved reviewed-plan transport.

use gent_protocol::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame};
use gent_runtime::catalog::declared_capabilities;
use gent_types::{AgentChatConversationId, ReviewedPlanId};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_reviewed_plan_authority() {
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
            .any(|capability| capability == REVIEWED_PLAN_CAPABILITY)
    );
    let error = runtime
        .reviewed_plan(ReviewedPlanFrame::ReviewRead {
            request_id: "request-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            plan_id: ReviewedPlanId("plan-1".into()),
        })
        .unwrap_err();
    assert_eq!(
        error,
        "reviewed plans are unavailable while gentd is observer-disabled"
    );
}
