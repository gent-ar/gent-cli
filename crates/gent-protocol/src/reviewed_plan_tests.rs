use super::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, ReviewedPlanFrameError};
use gent_types::{AgentChatConversationId, PlanRevision, ReviewedPlanId};
use serde_json::json;

#[test]
fn review_read_is_typed_and_capability_gated() {
    let frame = ReviewedPlanFrame::ReviewRead {
        request_id: "request-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        plan_id: ReviewedPlanId("plan-1".into()),
    };
    assert_eq!(frame.validate(), Ok(()));
    assert_eq!(REVIEWED_PLAN_CAPABILITY, "reviewed-plan-v1");
}

#[test]
fn start_implementation_cannot_accept_unknown_fields() {
    let frame = json!({
        "type": "startImplementation", "body": {
            "request": { "requestId": "request-1" }, "providerCommand": "never"
        }
    });
    assert!(serde_json::from_value::<ReviewedPlanFrame>(frame).is_err());
}

#[test]
fn rejection_binds_an_exact_plan_revision_and_digest() {
    let frame = ReviewedPlanFrame::Reject {
        request_id: "request-1".into(),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(2),
        plan_content_digest_sha256: "a".repeat(64),
    };
    assert_eq!(frame.validate(), Ok(()));
    let invalid = ReviewedPlanFrame::Reject {
        request_id: "request-1".into(),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(2),
        plan_content_digest_sha256: "A".repeat(64),
    };
    assert_eq!(invalid.validate(), Err(ReviewedPlanFrameError::InvalidPlan));
}

#[test]
fn response_correlation_is_required() {
    let frame = ReviewedPlanFrame::Rejected {
        request_id: "\n".into(),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(1),
    };
    assert_eq!(
        frame.validate(),
        Err(ReviewedPlanFrameError::InvalidIdentifier)
    );
}
