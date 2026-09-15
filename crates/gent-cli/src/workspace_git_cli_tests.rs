use gent_protocol::{
    Hello, Negotiated, WORKSPACE_GIT_CAPABILITY, WireFrame, WorkspaceGitFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX};
use serde_json::Value;
use tokio::net::UnixListener;

use super::{WorkspaceGitCommand, WorkspaceTarget, execute};
use crate::cli_error::CliError;

fn daemon(
    directory: &tempfile::TempDir,
    reply: impl Fn(WorkspaceGitFrame) -> Value + Send + 'static,
) {
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { .. })
        ));
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![WORKSPACE_GIT_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        while let Ok(frame) = read_json_frame::<_, WorkspaceGitFrame>(&mut stream).await {
            write_json_frame(&mut stream, &reply(frame)).await.unwrap();
        }
    });
}

#[tokio::test]
async fn an_unknown_workspace_is_reported_with_the_daemon_code_instead_of_a_decode_dump() {
    let directory = tempfile::tempdir().unwrap();
    daemon(
        &directory,
        |_| serde_json::json!({"type":"error","body":{"code":"workspaceNotFound","message":"workspace was not found"}}),
    );
    let error = execute(
        Some(directory.path().into()),
        true,
        WorkspaceGitCommand::Status {
            workspace: WorkspaceTarget {
                workspace_id: Some("workspace-missing".into()),
                path: None,
            },
        },
    )
    .await
    .unwrap_err();
    let error = error.downcast_ref::<CliError>().unwrap();
    assert_eq!(error.code(), Some("workspaceNotFound"));
    assert_eq!(
        error.to_string(),
        "workspace was not found [workspaceNotFound]"
    );
}

#[tokio::test]
async fn without_a_workspace_id_the_path_is_resolved_before_the_query() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let expected_path = workspace.path().display().to_string();
    daemon(&directory, move |frame| match frame {
        WorkspaceGitFrame::ResolveRequest {
            request_id,
            workspace_path,
        } => {
            assert_eq!(workspace_path, expected_path);
            serde_json::to_value(WorkspaceGitFrame::Resolved {
                request_id,
                workspace_id: "workspace-resolved".into(),
                canonical_path: workspace_path,
            })
            .unwrap()
        }
        WorkspaceGitFrame::SubReposRequest {
            request_id,
            workspace_id,
        } => serde_json::to_value(WorkspaceGitFrame::SubRepos {
            request_id,
            workspace_id,
            canonical_paths: vec!["/repo".into()],
        })
        .unwrap(),
        frame => panic!("unexpected {frame:?}"),
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        WorkspaceGitCommand::SubRepos {
            workspace: WorkspaceTarget {
                workspace_id: None,
                path: Some(workspace.path().into()),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(reply["body"]["workspaceId"], "workspace-resolved");
    assert_eq!(reply["body"]["canonicalPaths"][0], "/repo");
}
