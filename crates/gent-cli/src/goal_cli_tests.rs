use clap::Parser;
use gent_protocol::{
    GOAL_CAPABILITY, GoalFrame, Hello, Negotiated, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, CapabilitySet, GOAL_SCHEMA_VERSION, GoalBinding,
    GoalRecord, GoalStatus, HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN,
};
use tokio::net::UnixListener;

use super::{GoalCommand, StatusArgument, TransitionArgs, execute};
use crate::{Args, CommandLine};

#[test]
fn goal_commands_parse_only_typed_bound_fields() {
    let args = Args::try_parse_from([
        "gent",
        "goal",
        "transition",
        "--conversation-id",
        "conversation-1",
        "--run-id",
        "run-1",
        "--goal-id",
        "goal-1",
        "--expected-revision",
        "1",
        "--status",
        "completed",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Goal {
            action: GoalCommand::Transition(TransitionArgs {
                status: StatusArgument::Completed,
                expected_revision: 1,
                ..
            })
        })
    ));
}

#[tokio::test]
async fn observer_rejects_goal_before_a_goal_frame_is_sent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&GOAL_CAPABILITY.into())
        ));
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet::default(),
            }),
        )
        .await
        .unwrap();
    });
    let error = execute(
        Some(directory.path().into()),
        true,
        GoalCommand::List(super::ListArgs {
            conversation_id: "conversation-1".into(),
            request_id: Some("list-1".into()),
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
}

#[tokio::test]
async fn transition_uses_fresh_epoch_and_requires_exact_reply_binding() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![GOAL_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::StatusRequest
        ));
        write_frame(
            &mut stream,
            &WireFrame::Status(HostStatus {
                host_epoch: HostEpoch(9),
                protocol_min: PROTOCOL_MIN,
                protocol_max: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![GOAL_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let GoalFrame::Transition {
            request_id,
            transition,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected goal transition");
        };
        assert_eq!(transition.host_epoch, HostEpoch(9));
        write_json_frame(
            &mut stream,
            &GoalFrame::Transitioned {
                request_id,
                goal: GoalRecord {
                    schema_version: GOAL_SCHEMA_VERSION,
                    binding: transition.binding,
                    revision: 2,
                    status: GoalStatus::Completed,
                    summary: "Ship it".into(),
                },
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        GoalCommand::Transition(TransitionArgs {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            goal_id: "goal-1".into(),
            expected_revision: 1,
            status: StatusArgument::Completed,
            request_id: Some("transition-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(reply, GoalFrame::Transitioned { goal, .. } if goal.revision == 2));
}

#[test]
fn reply_correlation_rejects_a_different_goal_binding() {
    let request = GoalFrame::Read {
        request_id: "read-1".into(),
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        },
    };
    let reply = GoalFrame::Goal {
        request_id: "read-1".into(),
        binding: GoalBinding {
            goal_id: "goal-2".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        },
        goal: None,
    };
    assert!(!super::valid_reply(&request, &reply));
}
