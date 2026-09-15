use clap::Parser;
use gent_protocol::{
    GOAL_CAPABILITY, GoalFrame, GoalRejectionCode, Hello, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalStatusReason, PROTOCOL_MAX,
};
use tokio::net::UnixListener;

use super::{ControlArgs, GoalCommand, SetArgs, ShowArgs, execute};
use crate::{Args, CommandLine};

fn goal(revision: u64, status: GoalStatus) -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
        },
        revision,
        status,
        reason: GoalStatusReason::UserSet,
        objective: "Ship it".into(),
        note: None,
        time_used_seconds: 0,
        active_since: (status == GoalStatus::Active).then_some(10),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 10,
        updated_at: 10,
    }
}

async fn negotiate(listener: &UnixListener, capabilities: CapabilitySet) -> tokio::net::UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&GOAL_CAPABILITY.into())
    ));
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities,
        }),
    )
    .await
    .unwrap();
    stream
}

#[test]
fn goal_commands_parse_only_typed_conversation_fields() {
    let args = Args::try_parse_from([
        "gent",
        "goal",
        "set",
        "--conversation-id",
        "conversation-1",
        "--token-budget",
        "5000",
        "Ship the release",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Goal {
            action: GoalCommand::Set(SetArgs { token_budget: Some(5000), ref objective, .. })
        }) if objective == "Ship the release"
    ));
    for action in ["pause", "resume", "clear", "show"] {
        assert!(Args::try_parse_from(["gent", "goal", action, "--conversation-id", "c"]).is_ok());
    }
    assert!(
        Args::try_parse_from([
            "gent",
            "goal",
            "show",
            "--conversation-id",
            "c",
            "--run-id",
            "r"
        ])
        .is_err()
    );
}

#[tokio::test]
async fn observer_rejects_goal_before_a_goal_frame_is_sent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        negotiate(&listener, CapabilitySet::default()).await;
    });
    let error = execute(
        Some(directory.path().into()),
        true,
        GoalCommand::Show(ShowArgs {
            conversation_id: "conversation-1".into(),
            request_id: Some("show-1".into()),
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
}

#[tokio::test]
async fn pause_reads_the_current_revision_and_surfaces_typed_rejections() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        for attempt in 0..2 {
            let mut stream =
                negotiate(&listener, CapabilitySet(vec![GOAL_CAPABILITY.into()])).await;
            let GoalFrame::Read {
                request_id,
                conversation_id,
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("control must read the current goal first");
            };
            write_json_frame(
                &mut stream,
                &GoalFrame::Goal {
                    request_id,
                    conversation_id,
                    goal: Some(goal(4, GoalStatus::Active)),
                },
            )
            .await
            .unwrap();
            let GoalFrame::Pause {
                request_id,
                conversation_id,
                goal_id,
                expected_revision,
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected a pause intent");
            };
            assert_eq!((goal_id.as_str(), expected_revision), ("goal-1", 4));
            let reply = if attempt == 0 {
                GoalFrame::Goal {
                    request_id,
                    conversation_id,
                    goal: Some(goal(5, GoalStatus::Paused)),
                }
            } else {
                GoalFrame::Rejected {
                    request_id,
                    code: GoalRejectionCode::GoalRevisionMismatch,
                }
            };
            write_json_frame(&mut stream, &reply).await.unwrap();
        }
    });
    let pause = || {
        GoalCommand::Pause(ControlArgs {
            conversation_id: "conversation-1".into(),
            expected_revision: None,
            request_id: Some("pause-1".into()),
        })
    };
    let reply = execute(Some(directory.path().into()), true, pause())
        .await
        .unwrap();
    assert!(
        matches!(reply, GoalFrame::Goal { goal: Some(goal), .. } if goal.status == GoalStatus::Paused)
    );
    let error = execute(Some(directory.path().into()), true, pause())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("goalRevisionMismatch"));
}

#[test]
fn reply_correlation_rejects_another_conversation_or_goal() {
    let read = GoalFrame::Read {
        request_id: "read-1".into(),
        conversation_id: AgentChatConversationId("conversation-2".into()),
    };
    let reply = GoalFrame::Goal {
        request_id: "read-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        goal: None,
    };
    assert!(!super::valid_reply(&read, &reply));
    let report = GoalFrame::Report {
        request_id: "report-1".into(),
        goal_id: "goal-2".into(),
        outcome: gent_types::GoalReportOutcome::Complete,
        note: None,
    };
    let settled = GoalFrame::Goal {
        request_id: "report-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        goal: Some(goal(2, GoalStatus::Complete)),
    };
    assert!(!super::valid_reply(&report, &settled));
}
