//! Observer and approved-profile coverage for the durable `/goal` endpoint.

use gent_protocol::{GOAL_CAPABILITY, GoalFrame, GoalRejectionCode};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatSelection, GoalReportOutcome, GoalStatus, GoalStatusReason, ReceiptId,
};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_goals() {
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
            .any(|capability| capability == GOAL_CAPABILITY)
    );
    assert_eq!(
        runtime
            .goal(set_frame(AgentChatConversationId("conversation-1".into())))
            .unwrap_err(),
        "goals are unavailable while gentd is observer-disabled"
    );
}

#[test]
fn approved_chat_profile_sets_controls_and_reads_a_goal_through_the_facade() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = approved(directory.path());
    let conversation_id = seed_conversation(&runtime);
    let GoalFrame::Goal {
        goal: Some(goal), ..
    } = runtime.goal(set_frame(conversation_id.clone())).unwrap()
    else {
        panic!("expected the set goal");
    };
    assert_eq!(
        (goal.status, goal.token_budget),
        (GoalStatus::Active, Some(50_000))
    );
    assert_eq!(
        runtime.goal(set_frame(conversation_id.clone())).unwrap(),
        GoalFrame::Goal {
            request_id: "set-1".into(),
            conversation_id: conversation_id.clone(),
            goal: Some(goal.clone()),
        }
    );
    let stale = runtime
        .goal(GoalFrame::Pause {
            request_id: "pause-stale".into(),
            conversation_id: conversation_id.clone(),
            goal_id: goal.binding.goal_id.clone(),
            expected_revision: goal.revision + 5,
        })
        .unwrap();
    assert_eq!(
        stale,
        GoalFrame::Rejected {
            request_id: "pause-stale".into(),
            code: GoalRejectionCode::GoalRevisionMismatch,
        }
    );
    let GoalFrame::Goal {
        goal: Some(paused), ..
    } = runtime
        .goal(GoalFrame::Pause {
            request_id: "pause-1".into(),
            conversation_id: conversation_id.clone(),
            goal_id: goal.binding.goal_id.clone(),
            expected_revision: goal.revision,
        })
        .unwrap()
    else {
        panic!("expected the paused goal");
    };
    assert_eq!(
        (paused.status, paused.reason),
        (GoalStatus::Paused, GoalStatusReason::UserPaused)
    );
    assert_eq!(
        runtime
            .goal(GoalFrame::Report {
                request_id: "report-1".into(),
                goal_id: goal.binding.goal_id.clone(),
                outcome: GoalReportOutcome::Complete,
                note: None,
            })
            .unwrap(),
        GoalFrame::Rejected {
            request_id: "report-1".into(),
            code: GoalRejectionCode::GoalNoActiveTurn,
        }
    );
    clear_and_read(
        &runtime,
        &conversation_id,
        paused.revision,
        &goal.binding.goal_id,
    );
}

#[test]
fn goal_updates_reach_the_projection_snapshot_and_follow_stream() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = approved(directory.path());
    let conversation_id = seed_conversation(&runtime);
    let GoalFrame::Goal {
        goal: Some(goal), ..
    } = runtime.goal(set_frame(conversation_id.clone())).unwrap()
    else {
        panic!("expected the set goal");
    };
    let gent_protocol::AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } = runtime
        .agent_chat_projection(
            gent_protocol::AgentChatProjectionFrame::ConversationSnapshotRequest {
                request_id: "snapshot-1".into(),
                conversation_id: conversation_id.0.clone(),
                transcript_limit: 10,
                activity_limit: 10,
            },
        )
        .unwrap()
    else {
        panic!("expected a projection snapshot");
    };
    assert_eq!(snapshot.goal.as_ref(), Some(&goal));
    let followed = runtime
        .agent_chat_projection_follow(
            &conversation_id.0,
            &gent_protocol::ProjectionCursor { value: 0 },
        )
        .unwrap();
    assert!(followed.iter().any(|delta| {
        let gent_protocol::AgentChatProjectionDelta::Activity { fact, .. } = delta else {
            return false;
        };
        matches!(
            fact.as_ref(),
            gent_types::ConversationActivityFact::GoalUpdated { goal: published, .. }
                if *published == goal
        )
    }));
}

fn clear_and_read(
    runtime: &impl RuntimeApi,
    conversation_id: &AgentChatConversationId,
    revision: u64,
    goal_id: &str,
) {
    let GoalFrame::Goal {
        goal: Some(cleared),
        ..
    } = runtime
        .goal(GoalFrame::Clear {
            request_id: "clear-1".into(),
            conversation_id: conversation_id.clone(),
            goal_id: goal_id.into(),
            expected_revision: revision,
        })
        .unwrap()
    else {
        panic!("expected the cleared goal");
    };
    assert_eq!(cleared.status, GoalStatus::Cleared);
    assert_eq!(
        runtime
            .goal(GoalFrame::Read {
                request_id: "read-1".into(),
                conversation_id: conversation_id.clone(),
            })
            .unwrap(),
        GoalFrame::Goal {
            request_id: "read-1".into(),
            conversation_id: conversation_id.clone(),
            goal: None,
        }
    );
}

#[tokio::test]
async fn approved_chat_profile_dispatches_a_goal_over_the_typed_transport() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = approved(directory.path());
    let conversation_id = seed_conversation(&runtime);
    let (mut reader, mut writer) = tokio::io::duplex(8192);
    assert!(
        crate::goal_transport::dispatch(
            &mut writer,
            &runtime,
            &runtime.capabilities().unwrap(),
            &serde_json::to_value(set_frame(conversation_id)).unwrap(),
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        gent_protocol::read_json_frame::<_, GoalFrame>(&mut reader)
            .await
            .unwrap(),
        GoalFrame::Goal { goal: Some(goal), .. } if goal.status == GoalStatus::Active
    ));
}

pub(super) fn approved(path: &std::path::Path) -> crate::runtime_facade::RuntimeFacade {
    build_runtime(
        path,
        &RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat,
            RuntimeCapabilityFeature::AgentChatProjection,
        ]),
        CompatibilityAssessment::default(),
    )
    .unwrap()
}

pub(super) fn seed_conversation(runtime: &impl RuntimeApi) -> AgentChatConversationId {
    let created = runtime
        .agent_chat_intent(gent_protocol::AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("conversation-1".into()),
            workspace_path: ".".into(),
            selection: Some(AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            }),
            receipt_id: ReceiptId("receipt-1".into()),
        })
        .unwrap();
    let [
        gent_protocol::AgentChatIntentFrame::Created {
            conversation_id, ..
        },
    ] = created.as_slice()
    else {
        panic!("expected a created conversation");
    };
    conversation_id.clone()
}

fn set_frame(conversation_id: AgentChatConversationId) -> GoalFrame {
    GoalFrame::Set {
        request_id: "set-1".into(),
        conversation_id,
        objective: "Finish terminal support".into(),
        token_budget: Some(50_000),
    }
}

#[path = "goal_prompt_origin_tests.rs"]
mod prompt_origin;
