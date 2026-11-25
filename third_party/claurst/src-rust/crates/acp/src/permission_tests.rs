use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol_schema as acp;
use claurst_query::QueryEvent;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream, Lines};

use crate::connection::Connection;
use crate::sessions::SessionState;

struct Wire {
    connection: Arc<Connection>,
    session: Arc<SessionState>,
    frames: Lines<BufReader<DuplexStream>>,
}

fn wire() -> Wire {
    let (client, server) = tokio::io::duplex(1 << 16);
    let connection = Connection::new(server);
    let session = SessionState::new(
        acp::SessionId::new(format!("acp-{}", uuid::Uuid::new_v4())),
        std::env::temp_dir(),
    );
    super::spawn_forwarder(connection.clone(), session.clone());
    Wire {
        connection,
        session,
        frames: BufReader::new(client).lines(),
    }
}

async fn next_frame(wire: &mut Wire) -> Value {
    let line = tokio::time::timeout(Duration::from_secs(10), wire.frames.next_line())
        .await
        .expect("a frame arrives")
        .unwrap()
        .expect("the connection stays open");
    serde_json::from_str(&line).unwrap()
}

fn child_context(session: &SessionState) -> claurst_tools::ToolContext {
    claurst_tools::ToolContext {
        working_dir: session.cwd.clone(),
        permission_mode: claurst_core::config::PermissionMode::Default,
        permission_handler: Arc::new(super::AcpPermissionHandler),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: session.session_id.0.to_string(),
        file_history: session.file_history.clone(),
        current_turn: session.current_turn.clone(),
        non_interactive: false,
        mcp_manager: None,
        config: claurst_core::config::Config::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: Some(session.pending_permissions.clone()),
        permission_manager: None,
        user_question_tx: None,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_background_childs_permission_after_its_parent_turn_arrives_after_its_tool_start_and_ends_with_stop(
) {
    let mut wire = wire();
    let ctx = child_context(&wire.session);
    let stop = ctx.cancel_token.clone();
    let input = json!({"file_path": "notes.txt", "content": "hello"});
    let binding = claurst_tools::ToolCallBinding::new("toolu_child", "Write", &input);
    let child = tokio::spawn(claurst_tools::PARENT_TOOL_CALL.scope(
        "toolu_agent".to_string(),
        claurst_tools::TOOL_CALL.scope(binding, async move {
            ctx.check_permission("Write", "write notes.txt", false)
        }),
    ));

    let start = next_frame(&mut wire).await;
    let request = next_frame(&mut wire).await;

    assert_eq!(start["method"], "session/update");
    assert_eq!(start["params"]["update"]["sessionUpdate"], "tool_call");
    assert_eq!(start["params"]["update"]["toolCallId"], "toolu_child");
    assert_eq!(
        start["params"]["update"]["_meta"]["gentParentToolCallId"],
        "toolu_agent"
    );
    assert_eq!(request["method"], "session/request_permission");
    assert_eq!(request["params"]["toolCall"]["toolCallId"], "toolu_child");
    assert_eq!(
        request["params"]["toolCall"]["_meta"]["gentParentToolCallId"],
        "toolu_agent"
    );

    stop.cancel();
    let answer = tokio::time::timeout(Duration::from_secs(10), child)
        .await
        .expect("Stop releases the child's permission wait")
        .unwrap();
    assert!(answer.is_err());
}

#[tokio::test]
async fn a_childs_cancelled_tool_ends_cancelled_under_its_own_tool_call() {
    let mut wire = wire();
    let sink = crate::prompt::child_sink(wire.connection.clone(), wire.session.clone());

    sink(
        "toolu_agent".into(),
        QueryEvent::ToolStart {
            tool_name: "Bash".into(),
            tool_id: "toolu_child".into(),
            input_json: r#"{"command":"del calc.py"}"#.into(),
        },
    )
    .await;
    sink(
        "toolu_agent".into(),
        QueryEvent::ToolEnd {
            tool_name: "Bash".into(),
            tool_id: "toolu_child".into(),
            result: claurst_query::TOOL_CANCELLED_MSG.into(),
            is_error: true,
        },
    )
    .await;

    let start = next_frame(&mut wire).await;
    let end = next_frame(&mut wire).await;
    assert_eq!(start["params"]["update"]["toolCallId"], "toolu_child");
    assert_eq!(
        start["params"]["update"]["_meta"]["gentParentToolCallId"],
        "toolu_agent"
    );
    assert_eq!(end["params"]["update"]["sessionUpdate"], "tool_call_update");
    assert_eq!(end["params"]["update"]["toolCallId"], "toolu_child");
    assert_eq!(end["params"]["update"]["status"], "failed");
    assert_eq!(end["params"]["update"]["rawOutput"]["status"], "cancelled");
}
