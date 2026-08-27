use clap::Parser;
use gent_protocol::{
    Hello, Negotiated, ORCHESTRATION_CAPABILITY, OrchestrationFrame, WireFrame, read_frame,
    write_frame,
};
use gent_types::{AgentChatConversationId, CapabilitySet, PROTOCOL_MAX};
use tokio::net::UnixListener;

use super::{OrchestrationCommand, ReadArgs, execute};
use crate::{Args, CommandLine};

#[test]
fn orchestration_commands_parse_bounded_file_or_scope_arguments() {
    let args = Args::try_parse_from([
        "gent",
        "orchestration",
        "fanout",
        "--graph-json",
        "graph.json",
        "--request-id",
        "fanout-1",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Orchestration {
            action: OrchestrationCommand::Fanout(_)
        })
    ));
    assert!(
        Args::try_parse_from([
            "gent",
            "orchestration",
            "read",
            "--conversation-id",
            "c1",
            "--graph-id",
            "g1"
        ])
        .is_ok()
    );
}

#[tokio::test]
async fn observer_refuses_before_an_orchestration_frame_is_sent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|item| item == ORCHESTRATION_CAPABILITY)
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
        OrchestrationCommand::Read(ReadArgs {
            conversation: "conversation-1".into(),
            graph: "graph-1".into(),
            request: Some("read-1".into()),
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
}

#[test]
fn reply_correlation_rejects_a_different_graph_scope() {
    let request = OrchestrationFrame::GraphRead {
        request_id: "read-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        graph_id: "graph-1".into(),
    };
    let reply = OrchestrationFrame::Graph {
        request_id: "read-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        graph_id: "graph-2".into(),
        graph: None,
    };
    assert!(!super::valid_reply(&request, &reply));
}
