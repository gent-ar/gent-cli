use std::sync::Arc;

use agent_client_protocol_schema as acp;
use claurst_core::tasks::{global_registry, BackgroundTask, TaskStatus};
use tokio::sync::broadcast::error::RecvError;

use crate::connection::Connection;
use crate::prompt::send_session_update;
use crate::sessions::{SessionRegistry, SessionState};

pub(crate) fn spawn_end_reporter(connection: Arc<Connection>, sessions: Arc<SessionRegistry>) {
    let mut ends = global_registry().ends();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = connection.closed_token.cancelled() => return,
                event = ends.recv() => match event {
                    Ok(task) => report_end(&connection, &sessions, &task).await,
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
        }
    });
}

pub(crate) async fn report_end(
    connection: &Arc<Connection>,
    sessions: &SessionRegistry,
    task: &BackgroundTask,
) {
    let Some(session) = task
        .session_id
        .as_deref()
        .and_then(|id| sessions.get(&acp::SessionId::new(id)))
    else {
        return;
    };
    let mut launches = session.background_launches.lock().await;
    if launches.remove(&task.id) {
        send_session_update(connection, &session.session_id, end_update(task)).await;
    }
}

pub(crate) async fn report_tool_result(
    connection: &Arc<Connection>,
    session: &SessionState,
    tool_id: &str,
    result: &str,
    is_error: bool,
) {
    let mut launches = session.background_launches.lock().await;
    let launched = global_registry().list().into_iter().find(|task| {
        task.session_id.as_deref() == Some(&*session.session_id.0)
            && (task.id == tool_id || task.tool_call_id.as_deref() == Some(tool_id))
    });
    let update = match launched {
        Some(task) if task.is_running() => {
            launches.insert(task.id);
            result_update(tool_id, acp::ToolCallStatus::InProgress, result)
        }
        Some(task) => end_update(&task),
        None if result == claurst_query::TOOL_CANCELLED_MSG => cancelled_update(tool_id, result),
        None if is_error => result_update(tool_id, acp::ToolCallStatus::Failed, result),
        None => result_update(tool_id, acp::ToolCallStatus::Completed, result),
    };
    send_session_update(connection, &session.session_id, update).await;
}

fn result_update(tool_id: &str, status: acp::ToolCallStatus, result: &str) -> acp::SessionUpdate {
    let raw_output = serde_json::from_str::<serde_json::Value>(result)
        .unwrap_or_else(|_| serde_json::Value::String(result.to_string()));
    let fields = acp::ToolCallUpdateFields::new()
        .status(status)
        .content(vec![text_content(result)])
        .raw_output(Some(raw_output));
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new(tool_id),
        fields,
    ))
}

fn cancelled_update(tool_id: &str, result: &str) -> acp::SessionUpdate {
    let fields = acp::ToolCallUpdateFields::new()
        .status(acp::ToolCallStatus::Failed)
        .content(vec![text_content(result)])
        .raw_output(Some(serde_json::json!({ "status": "cancelled" })));
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new(tool_id),
        fields,
    ))
}

fn end_update(task: &BackgroundTask) -> acp::SessionUpdate {
    let status = match task.status {
        TaskStatus::Running => acp::ToolCallStatus::InProgress,
        TaskStatus::Completed => acp::ToolCallStatus::Completed,
        TaskStatus::Failed(_) | TaskStatus::Cancelled => acp::ToolCallStatus::Failed,
    };
    let mut fields = acp::ToolCallUpdateFields::new()
        .status(status)
        .raw_output(Some(serde_json::json!({ "status": task.status.to_string() })));
    if !task.output.is_empty() {
        fields = fields.content(vec![text_content(&task.output.join("\n"))]);
    }
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new(task.tool_call_id.as_deref().unwrap_or(&task.id)),
        fields,
    ))
}

fn text_content(text: &str) -> acp::ToolCallContent {
    acp::ToolCallContent::Content(acp::Content::new(acp::ContentBlock::Text(
        acp::TextContent::new(text),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_query::QueryEvent;
    use serde_json::Value;
    use tokio::io::{duplex, AsyncReadExt, DuplexStream};
    use tokio::sync::mpsc;

    struct Wire {
        connection: Arc<Connection>,
        client: DuplexStream,
        sessions: SessionRegistry,
        session: Arc<SessionState>,
    }

    fn wire() -> Wire {
        let (client, server) = duplex(1 << 16);
        let sessions = SessionRegistry::new();
        let session = SessionState::new(
            acp::SessionId::new(format!("acp-{}", uuid::Uuid::new_v4())),
            std::env::temp_dir(),
        );
        sessions.insert(session.clone());
        Wire {
            connection: Connection::new(server),
            client,
            sessions,
            session,
        }
    }

    fn launch_in(session: &SessionState) -> String {
        let mut task = BackgroundTask::new("bg: ping");
        task.id = format!("toolu_{}", uuid::Uuid::new_v4().simple());
        task.session_id = Some(session.session_id.0.to_string());
        global_registry().register(task)
    }

    async fn forward_tool_call(wire: &Wire, tool_id: &str) {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(QueryEvent::ToolStart {
            tool_name: "Bash".into(),
            tool_id: tool_id.into(),
            input_json: r#"{"command":"ping -n 60 127.0.0.1","run_in_background":true}"#.into(),
        })
        .unwrap();
        tx.send(QueryEvent::ToolEnd {
            tool_name: "Bash".into(),
            tool_id: tool_id.into(),
            result: "Command started in background.".into(),
            is_error: false,
        })
        .unwrap();
        drop(tx);
        crate::prompt::forward_events(wire.connection.clone(), wire.session.clone(), rx).await;
    }

    async fn report(wire: &Wire, tool_id: &str) {
        let task = global_registry().get(tool_id).unwrap();
        report_end(&wire.connection, &wire.sessions, &task).await;
    }

    async fn updates(wire: Wire) -> Vec<Value> {
        let Wire {
            connection,
            mut client,
            ..
        } = wire;
        drop(connection);
        let mut text = String::new();
        client.read_to_string(&mut text).await.unwrap();
        text.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["params"]["update"].clone())
            .collect()
    }

    fn sequence(updates: &[Value]) -> Vec<(&str, &str)> {
        updates
            .iter()
            .map(|u| {
                (
                    u["sessionUpdate"].as_str().unwrap(),
                    u["status"].as_str().unwrap_or("-"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_background_launch_stays_running_until_its_command_completes() {
        let wire = wire();
        let tool_id = launch_in(&wire.session);

        forward_tool_call(&wire, &tool_id).await;
        global_registry().append_output(&tool_id, "Reply from 127.0.0.1");
        global_registry().complete(&tool_id);
        report(&wire, &tool_id).await;

        let updates = updates(wire).await;
        assert_eq!(
            sequence(&updates),
            [
                ("tool_call", "in_progress"),
                ("tool_call_update", "in_progress"),
                ("tool_call_update", "completed"),
            ]
        );
        assert!(updates.iter().all(|u| u["toolCallId"] == tool_id.as_str()));
        assert_eq!(updates[2]["content"][0]["content"]["text"], "Reply from 127.0.0.1");
    }

    #[tokio::test]
    async fn a_background_agent_uses_its_tool_call_for_updates_and_session_for_cancellation() {
        let wire = wire();
        let mut task = BackgroundTask::new("subagent: verify");
        let task_id = task.id.clone();
        let tool_id = format!("toolu_{}", uuid::Uuid::new_v4().simple());
        task.session_id = Some(wire.session.session_id.0.to_string());
        task.tool_call_id = Some(tool_id.clone());
        let token = tokio_util::sync::CancellationToken::new();
        task.cancel_token = Some(token.clone());
        global_registry().register(task);

        forward_tool_call(&wire, &tool_id).await;
        global_registry().cancel_session(&wire.session.session_id.0);
        assert!(token.is_cancelled());
        assert!(global_registry().get(&task_id).unwrap().is_running());
        global_registry().update_status(&task_id, TaskStatus::Cancelled);
        report(&wire, &task_id).await;

        let updates = updates(wire).await;
        assert_eq!(
            sequence(&updates),
            [
                ("tool_call", "in_progress"),
                ("tool_call_update", "in_progress"),
                ("tool_call_update", "failed")
            ]
        );
        assert!(updates.iter().all(|u| u["toolCallId"] == tool_id.as_str()));
        assert_eq!(updates[2]["rawOutput"]["status"], "cancelled");
    }

    #[tokio::test]
    async fn a_background_agent_reports_completion_against_its_originating_tool_call() {
        let wire = wire();
        let mut task = BackgroundTask::new("subagent: verify");
        let task_id = task.id.clone();
        let tool_id = format!("toolu_{}", uuid::Uuid::new_v4().simple());
        task.session_id = Some(wire.session.session_id.0.to_string());
        task.tool_call_id = Some(tool_id.clone());
        global_registry().register(task);

        forward_tool_call(&wire, &tool_id).await;
        global_registry().append_output(&task_id, "finished");
        global_registry().complete(&task_id);
        report(&wire, &task_id).await;

        let updates = updates(wire).await;
        assert_eq!(
            sequence(&updates),
            [
                ("tool_call", "in_progress"),
                ("tool_call_update", "in_progress"),
                ("tool_call_update", "completed")
            ]
        );
        assert!(updates.iter().all(|u| u["toolCallId"] == tool_id.as_str()));
        assert_eq!(updates[2]["content"][0]["content"]["text"], "finished");
    }

    #[tokio::test]
    async fn a_cancelled_background_command_ends_failed_as_cancelled() {
        let wire = wire();
        let tool_id = launch_in(&wire.session);

        forward_tool_call(&wire, &tool_id).await;
        global_registry().cancel_session(&wire.session.session_id.0);
        report(&wire, &tool_id).await;

        let updates = updates(wire).await;
        assert_eq!(
            sequence(&updates),
            [
                ("tool_call", "in_progress"),
                ("tool_call_update", "in_progress"),
                ("tool_call_update", "failed"),
            ]
        );
        assert_eq!(updates[2]["rawOutput"]["status"], "cancelled");
    }

    #[tokio::test]
    async fn a_command_that_ends_before_its_launch_is_reported_ends_once() {
        let wire = wire();
        let tool_id = launch_in(&wire.session);

        global_registry().complete(&tool_id);
        report(&wire, &tool_id).await;
        forward_tool_call(&wire, &tool_id).await;
        report(&wire, &tool_id).await;

        let updates = updates(wire).await;
        assert_eq!(
            sequence(&updates),
            [
                ("tool_call", "in_progress"),
                ("tool_call_update", "completed"),
            ]
        );
    }
}
