use gent_protocol::{
    AGENT_CHAT_SESSIONS_CAPABILITY, AGENT_CHAT_SIDE_QUESTION_CAPABILITY, Hello, Negotiated,
    WireFrame, agent_chat_commands::AgentChatCommandRejectionFrame,
    agent_chat_interrupt::InterruptRejectionFrame, read_frame, read_json_frame,
    workspace_unavailable::WorkspaceRejectionFrame, write_frame, write_json_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    process::Command,
};

#[cfg(unix)]
fn serve(directory: &TempDir, reply: Value) -> tokio::task::JoinHandle<Value> {
    let listener =
        tokio::net::UnixListener::bind(gent_types::local_socket_path(directory.path())).unwrap();
    tokio::spawn(async move { answer(listener.accept().await.unwrap().0, reply).await })
}

#[cfg(windows)]
fn serve(directory: &TempDir, reply: Value) -> tokio::task::JoinHandle<Value> {
    let server = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .create(gent_types::windows_pipe_name(directory.path()))
        .unwrap();
    tokio::spawn(async move {
        server.connect().await.unwrap();
        answer(server, reply).await
    })
}

async fn answer<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, reply: Value) -> Value {
    assert!(matches!(
        read_frame(&mut stream).await.unwrap(),
        WireFrame::Hello(Hello { .. })
    ));
    write_frame(
        &mut stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                AGENT_CHAT_SIDE_QUESTION_CAPABILITY.into(),
                AGENT_CHAT_SESSIONS_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    let request: Value = read_json_frame(&mut stream).await.unwrap();
    write_json_frame(&mut stream, &reply).await.unwrap();
    request
}

async fn gent(directory: &TempDir, reply: Value, args: &[&str]) -> (Option<i32>, String, String) {
    let daemon = serve(directory, reply);
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
        ])
        .args(args)
        .output()
        .await
        .unwrap();
    daemon.await.unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

async fn ask(reply: Value) -> (Option<i32>, String, String) {
    let directory = tempfile::tempdir().unwrap();
    gent(
        &directory,
        reply,
        &[
            "side-question",
            "ask",
            "--conversation-id",
            "conversation-1",
            "what changed?",
        ],
    )
    .await
}

fn wire<T: serde::Serialize>(frame: &T) -> Value {
    serde_json::to_value(frame).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_side_question_blocked_by_the_active_turn_prints_the_code_instead_of_a_decode_error() {
    let (code, stdout, stderr) = ask(json!({"type": "error", "body": {
        "code": "sideQuestionBlockedByActiveTurn",
        "message": "this provider answers side questions only between turns"
    }}))
    .await;
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr.trim_end(),
        "gent: this provider answers side questions only between turns [sideQuestionBlockedByActiveTurn]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_workspace_rejection_names_the_folder() {
    let (code, _, stderr) = ask(wire(&WorkspaceRejectionFrame::Error {
        code: "workspaceUnavailable".into(),
        message: "the workspace folder /gone does not exist or is not a directory".into(),
        workspace: gent_protocol::workspace_unavailable::WorkspaceUnavailable {
            path: "/gone".into(),
        },
    }))
    .await;
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr.trim_end(),
        "gent: the workspace folder /gone does not exist or is not a directory [workspaceUnavailable]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_turn_rejection_says_its_background_work_is_still_running() {
    let (code, _, stderr) = ask(wire(&InterruptRejectionFrame::Error {
        code: "interruptTurnEnded".into(),
        message: "the interrupted turn already ended".into(),
        interrupt: gent_protocol::agent_chat_interrupt::InterruptTurnEnded {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            turn_phase: gent_types::DurableTurnPhase::Completed,
            background_work_running: true,
        },
    }))
    .await;
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr.trim_end(),
        "gent: the interrupted turn already ended; its background work is still running [interruptTurnEnded]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slash_command_rejection_keeps_the_daemon_sentence_and_code() {
    let (code, _, stderr) = ask(wire(&AgentChatCommandRejectionFrame::Error {
        code: "commandArgumentsInvalid".into(),
        message: "/model needs a model name".into(),
        command: gent_types::AgentChatCommandRejectionDetails {
            name: "model".into(),
            hint: Some("a model name".into()),
            reason: None,
            use_instead: None,
        },
    }))
    .await;
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr.trim_end(),
        "gent: /model needs a model name [commandArgumentsInvalid]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_reply_that_is_a_missing_workspace_exits_as_not_found() {
    let directory = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = gent(
        &directory,
        json!({"type": "error", "body": {
            "code": "workspaceNotFound",
            "message": "workspace was not found"
        }}),
        &["sessions", "list", "workspace-1"],
    )
    .await;
    assert_eq!(code, Some(4));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr.trim_end(),
        "gent: workspace was not found [workspaceNotFound]"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrecognized_reply_is_a_sentence_not_a_serde_error() {
    let (code, _, stderr) = ask(json!({"type": "somethingNew", "body": {}})).await;
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr.trim_end(),
        "gent: gentd returned a reply gent does not recognize"
    );
}
