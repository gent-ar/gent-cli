//! Observer-safety tests for the reserved reviewed-plan transport.

use gent_protocol::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame};
use gent_runtime::catalog::{declared_capabilities, declared_capabilities_with_agent_chat};
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

#[test]
fn approved_chat_persistence_profile_advertises_and_reads_reviewed_plans() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities_with_agent_chat(true),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(
        runtime
            .capabilities()
            .unwrap()
            .0
            .contains(&REVIEWED_PLAN_CAPABILITY.into())
    );
    assert!(matches!(
        runtime
            .reviewed_plan(ReviewedPlanFrame::ReviewRead {
                request_id: "request-1".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                plan_id: ReviewedPlanId("plan-1".into()),
            })
            .unwrap(),
        ReviewedPlanFrame::Review { plan: None, .. }
    ));
}
