use std::io;

use serde_json::json;

use super::{CliError, Failure, describe};

#[test]
fn daemon_error_frames_keep_their_typed_code() {
    let error = CliError::from_reply(&json!({
        "type": "error",
        "body": {
            "code": "selectionSwitchBlockedByActiveTurn",
            "message": "the current turn must settle before changing its model or provider"
        }
    }))
    .unwrap();
    assert_eq!(
        describe(&error),
        (
            Failure::Rejected,
            "the current turn must settle before changing its model or provider [selectionSwitchBlockedByActiveTurn]"
                .into()
        )
    );
}

#[test]
fn missing_conversations_and_workspaces_exit_as_not_found() {
    for code in ["conversationNotFound", "workspaceNotFound"] {
        let error = CliError::daemon(code, "no such item");
        assert_eq!(describe(&error).0, Failure::NotFound, "{code}");
        assert_eq!(Failure::NotFound.exit_code(), 4);
    }
    assert_eq!(
        describe(&CliError::daemon("selectionEffortUnavailable", "no")).0,
        Failure::Rejected
    );
}

#[test]
fn non_error_frames_are_not_errors() {
    assert!(CliError::from_reply(&json!({"type": "status"})).is_none());
}

#[test]
fn io_failures_render_without_debug_structures() {
    let (failure, message) = describe(&io::Error::from(io::ErrorKind::UnexpectedEof));
    assert_eq!(failure, Failure::Rejected);
    assert_eq!(message, "gentd closed the connection before replying");
    let (_, message) = describe(&io::Error::from(io::ErrorKind::NotFound));
    assert!(
        !message.contains("Os {") && !message.contains("Kind("),
        "{message}"
    );
}

#[test]
fn every_failure_has_a_distinct_documented_exit_code() {
    let codes = [
        Failure::Rejected,
        Failure::Unavailable,
        Failure::NotFound,
        Failure::ConsentRequired,
        Failure::TurnFailed,
        Failure::TurnInterrupted,
    ]
    .map(Failure::exit_code);
    assert_eq!(codes, [1, 3, 4, 5, 6, 7]);
}
