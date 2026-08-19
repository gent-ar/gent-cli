//! Observer-safety tests for the reserved reviewed-plan transport.

use gent_protocol::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{AgentChatConversationId, ReviewedPlanId};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_reviewed_plan_authority() {
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
fn chat_persistence_profile_keeps_reviewed_plans_unadvertised_until_lifecycle_authority() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &RuntimeCapabilityProfile::new([RuntimeCapabilityFeature::AgentChat]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(
        !runtime
            .capabilities()
            .unwrap()
            .0
            .contains(&REVIEWED_PLAN_CAPABILITY.into())
    );
    assert_eq!(
        runtime
            .reviewed_plan(ReviewedPlanFrame::ReviewRead {
                request_id: "request-1".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                plan_id: ReviewedPlanId("plan-1".into()),
            })
            .unwrap_err(),
        "reviewed plans are unavailable while gentd is observer-disabled"
    );
}
