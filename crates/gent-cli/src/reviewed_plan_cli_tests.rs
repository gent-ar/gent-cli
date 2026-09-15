use clap::Parser;
use gent_protocol::{
    Hello, Negotiated, REVIEWED_PLAN_CAPABILITY, WireFrame, read_frame, write_frame,
};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, CapabilitySet,
    PROTOCOL_MAX,
};
use tokio::net::UnixListener;

use super::{ContextArgument, ReviewedPlanCommand, StartArgs, conversions, execute};
use crate::chat_cli::Provider;
use crate::{Args, CommandLine};

fn planning() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "haiku".into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Plan,
    }
}

#[test]
fn start_needs_only_the_conversation_and_optional_implementation_choices() {
    let args = Args::try_parse_from([
        "gent",
        "plan",
        "start",
        "--conversation-id",
        "conversation-1",
        "--context",
        "clear",
        "--provider",
        "gent",
        "--model",
        "local-model",
    ])
    .unwrap();
    assert!(matches!(
        args.command,
        Some(CommandLine::Plan {
            action: ReviewedPlanCommand::Start(StartArgs {
                context: ContextArgument::Clear,
                provider: Some(Provider::Gent),
                ..
            })
        })
    ));
    assert!(Args::try_parse_from(["gent", "plan", "reject", "--conversation-id", "c"]).is_ok());
    assert!(Args::try_parse_from(["gent", "plan", "review", "--conversation-id", "c"]).is_ok());
}

#[test]
fn implementation_keeps_the_planning_selection_unless_overridden() {
    assert_eq!(
        conversions::implementation_selection(
            &planning(),
            None,
            None,
            Some(gent_types::AgentChatEffort::High)
        )
        .unwrap(),
        AgentChatSelection {
            effort: AgentChatEffort::High,
            ..planning()
        }
    );
    assert!(
        conversions::implementation_selection(&planning(), Some(Provider::Codex), None, None)
            .is_err()
    );
    assert_eq!(
        conversions::implementation_selection(
            &planning(),
            Some(Provider::Codex),
            Some("gpt-6-astra".into()),
            None
        )
        .unwrap()
        .provider,
        AgentChatProvider::Codex
    );
}

#[tokio::test]
async fn observer_mode_rejects_before_any_plan_frame_is_sent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|item| item == REVIEWED_PLAN_CAPABILITY)
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
        ReviewedPlanCommand::Review(super::ReviewArgs {
            conversation_id: "conversation-1".into(),
            plan_id: None,
        }),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("observer mode"));
}
