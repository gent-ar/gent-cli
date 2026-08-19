//! Observer and approved-profile coverage for the durable `/goal` endpoint.

use gent_protocol::{GOAL_CAPABILITY, GoalFrame};
use gent_runtime::catalog::{declared_capabilities, declared_capabilities_with_agent_chat};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord, GoalStatus, ReceiptId,
};

use crate::{CompatibilityAssessment, api::RuntimeApi, build_runtime};

#[test]
fn observer_neither_advertises_nor_accepts_goals() {
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
            .any(|capability| capability == GOAL_CAPABILITY)
    );
    assert_eq!(
        runtime.goal(create_frame(test_binding())).unwrap_err(),
        "goals are unavailable while gentd is observer-disabled"
    );
}

#[test]
fn approved_chat_profile_persists_and_reads_a_goal_through_the_facade() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities_with_agent_chat(true),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let binding = seed_conversation(&runtime);
    let created = runtime.goal(create_frame(binding)).unwrap();
    let GoalFrame::Created { goal, .. } = created else {
        panic!("expected a created goal");
    };
    assert_eq!(goal.status, GoalStatus::Active);
    let read = runtime
        .goal(GoalFrame::Read {
            request_id: "read-1".into(),
            binding: goal.binding.clone(),
        })
        .unwrap();
    assert!(matches!(read, GoalFrame::Goal { goal: Some(_), .. }));
}

#[tokio::test]
async fn approved_chat_profile_dispatches_a_goal_over_the_typed_transport() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = build_runtime(
        directory.path(),
        &declared_capabilities_with_agent_chat(true),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let binding = seed_conversation(&runtime);
    let request = create_frame(binding);
    let (mut reader, mut writer) = tokio::io::duplex(4096);
    assert!(
        crate::goal_transport::dispatch(
            &mut writer,
            &runtime,
            &runtime.capabilities().unwrap(),
            &serde_json::to_value(request).unwrap(),
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        gent_protocol::read_json_frame::<_, GoalFrame>(&mut reader)
            .await
            .unwrap(),
        GoalFrame::Created { goal, .. } if goal.status == GoalStatus::Active
    ));
}

fn seed_conversation(runtime: &impl RuntimeApi) -> GoalBinding {
    let created = runtime
        .agent_chat_intent(gent_protocol::AgentChatIntentFrame::CreateConversation {
            request_id: AgentChatRequestId("conversation-1".into()),
            workspace_path: ".".into(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            receipt_id: ReceiptId("receipt-1".into()),
        })
        .unwrap();
    let [
        gent_protocol::AgentChatIntentFrame::Created {
            conversation_id,
            run_id,
            ..
        },
    ] = created.as_slice()
    else {
        panic!("expected a created conversation");
    };
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: conversation_id.clone(),
        run_id: run_id.clone(),
    }
}

fn create_frame(binding: GoalBinding) -> GoalFrame {
    GoalFrame::Create {
        request_id: "create-1".into(),
        goal: GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding,
            revision: 1,
            status: GoalStatus::Active,
            summary: "Finish terminal support".into(),
        },
    }
}

fn test_binding() -> GoalBinding {
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
        run_id: gent_types::AgentChatRunId("run-1".into()),
    }
}
