use gent_ports::AgentChatSideQuestionLedger;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROJECTION_CAPABILITY,
    AGENT_CHAT_SIDE_QUESTION_CAPABILITY, AgentChatProjectionFrame, AgentChatSideQuestionFrame,
    Hello, ProjectionCursor, WireFrame,
    agent_chat_commands::{AGENT_CHAT_COMMANDS_CAPABILITY, AgentChatCommandFrame},
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatSideQuestionFailureCode,
    AgentChatSideQuestionOutcome, CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{DuplexStream, duplex};

use super::{RuntimeFacade, facade, root, root_with_selection, running};

pub(super) async fn connect(runtime: &RuntimeFacade) -> DuplexStream {
    let (mut client, server) = duplex(1024 * 1024);
    tokio::spawn(crate::transport::serve_connection(server, runtime.clone()));
    let capabilities = [
        AGENT_CHAT_INTENTS_CAPABILITY,
        AGENT_CHAT_PROJECTION_CAPABILITY,
        AGENT_CHAT_SIDE_QUESTION_CAPABILITY,
        AGENT_CHAT_COMMANDS_CAPABILITY,
    ];
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(capabilities.map(str::to_owned).to_vec()),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(negotiated) = read_frame(&mut client).await.unwrap() else {
        panic!("the daemon negotiates");
    };
    assert_eq!(negotiated.capabilities.0.len(), capabilities.len());
    client
}

async fn exchange(client: &mut DuplexStream, request: &impl Serialize) -> Value {
    write_json_frame(client, request).await.unwrap();
    read_json_frame(client).await.unwrap()
}

fn ask(conversation: &AgentChatConversationId, key: &str) -> AgentChatSideQuestionFrame {
    AgentChatSideQuestionFrame::AskSideQuestion {
        request_id: format!("ask-{key}"),
        receipt_id: format!("ask-receipt-{key}"),
        conversation_id: conversation.0.clone(),
        question: "what is it doing?".into(),
    }
}

async fn settled_fact(runtime: &RuntimeFacade, conversation: &AgentChatConversationId) -> Value {
    let mut follower = connect(runtime).await;
    write_json_frame(
        &mut follower,
        &AgentChatProjectionFrame::FollowConversation {
            request_id: "follow".into(),
            conversation_id: conversation.0.clone(),
            after_cursor: ProjectionCursor { value: 0 },
        },
    )
    .await
    .unwrap();
    loop {
        let delta: Value = read_json_frame(&mut follower).await.unwrap();
        let fact = &delta["body"]["delta"]["body"]["fact"];
        if fact["type"] == "sideQuestionSettled" {
            return json!({
                "status": fact["status"],
                "failureCode": fact["failureCode"],
            });
        }
    }
}

#[tokio::test]
async fn a_claurst_side_question_during_a_turn_is_refused_before_it_is_accepted() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "busy", ".");
    running(&runtime, &conversation, "busy");
    let mut client = connect(&runtime).await;
    let catalog = exchange(
        &mut client,
        &AgentChatCommandFrame::ReadCommandCatalog {
            request_id: AgentChatRequestId("catalog".into()),
            conversation_id: Some(conversation.clone()),
            workspace_path: None,
            refresh: false,
        },
    )
    .await;
    let btw = catalog["body"]["catalog"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["name"] == "btw")
        .unwrap()
        .clone();
    assert_eq!(btw["availability"]["blockedWhileTurnActive"], json!(true));
    assert_eq!(
        exchange(&mut client, &ask(&conversation, "busy")).await,
        json!({"type": "error", "body": {
            "code": "sideQuestionBlockedByActiveTurn",
            "message": "this provider answers side questions only between turns"
        }})
    );
    let listed = exchange(
        &mut client,
        &AgentChatSideQuestionFrame::ListSideQuestions {
            request_id: "list".into(),
            conversation_id: conversation.0.clone(),
        },
    )
    .await;
    assert_eq!(listed["body"]["sideQuestions"], json!([]));
}

#[tokio::test]
async fn an_idle_claurst_side_question_settles_with_a_typed_failure_code() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "idle", ".");
    let mut client = connect(&runtime).await;
    let asked = exchange(&mut client, &ask(&conversation, "idle")).await;
    assert_eq!(asked["type"], "asked");
    assert_eq!(
        settled_fact(&runtime, &conversation).await,
        json!({"status": "failed", "failureCode": "providerUnavailable"})
    );
}

#[tokio::test]
async fn a_side_question_that_loses_the_race_to_a_turn_settles_as_provider_busy() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "raced", ".");
    let ledger = runtime.conversation_links().unwrap();
    let asked = ledger
        .ask_agent_chat_side_question(
            &gent_types::AgentChatSideQuestion {
                request_id: AgentChatRequestId("ask-raced".into()),
                receipt_id: gent_types::ReceiptId("ask-receipt-raced".into()),
                host_epoch: gent_types::HostEpoch(1),
                conversation_id: conversation.clone(),
                question: "what now?".into(),
                created_at_unix_ms: 1,
            },
            "side-raced",
        )
        .unwrap();
    ledger
        .complete_agent_chat_side_question(
            &asked.record.side_question_id,
            &AgentChatSideQuestionOutcome::Failed {
                code: AgentChatSideQuestionFailureCode::ProviderBusy,
                reason: "the local runtime is running a turn".into(),
            },
        )
        .unwrap();
    assert_eq!(
        settled_fact(&runtime, &conversation).await,
        json!({"status": "failed", "failureCode": "providerBusy"})
    );
}

#[tokio::test]
async fn only_a_gent_local_conversation_marks_btw_blocked_while_a_turn_runs() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let mut client = connect(&runtime).await;
    let mut blocked = Vec::new();
    for (provider, model) in [
        (gent_types::AgentChatProvider::Claude, "haiku"),
        (gent_types::AgentChatProvider::Codex, "gpt-5.1-codex-mini"),
        (gent_types::AgentChatProvider::Claurst, "qwen3-1-7b-q4-k-m"),
    ] {
        let conversation = root_with_selection(
            &runtime,
            model,
            ".",
            gent_types::AgentChatSelection {
                provider,
                model: model.into(),
                effort: gent_types::AgentChatEffort::Medium,
                mode: gent_types::AgentChatMode::Agent,
            },
        );
        let catalog = exchange(
            &mut client,
            &AgentChatCommandFrame::ReadCommandCatalog {
                request_id: AgentChatRequestId(format!("catalog-{model}")),
                conversation_id: Some(conversation),
                workspace_path: None,
                refresh: false,
            },
        )
        .await;
        let btw = catalog["body"]["catalog"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["name"] == "btw")
            .unwrap()["availability"]["blockedWhileTurnActive"]
            .clone();
        blocked.push((catalog["body"]["catalog"]["scope"]["provider"].clone(), btw));
    }
    assert_eq!(
        blocked,
        [
            (json!("claude"), json!(false)),
            (json!("codex"), json!(false)),
            (json!("claurst"), json!(true)),
        ]
    );
}
