use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROJECTION_CAPABILITY,
    AGENT_CHAT_PROVIDER_WARM_CAPABILITY, AgentChatIntentFrame, AgentChatProjectionFrame,
    AgentChatProviderWarm, Hello, PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame,
    WORKSPACE_GIT_CAPABILITY, WireFrame, WorkspaceGitFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_runtime::catalog::{RuntimeCapabilityFeature, RuntimeCapabilityProfile};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN, PermissionMode,
    PolicyRecord, PolicyScope, ReceiptId,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{DuplexStream, duplex};

use crate::{CompatibilityAssessment, RuntimeFacade, build_runtime};

async fn connect(runtime: &RuntimeFacade) -> DuplexStream {
    let (mut client, server) = duplex(1024 * 1024);
    tokio::spawn(crate::transport::serve_connection(server, runtime.clone()));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(
                [
                    AGENT_CHAT_INTENTS_CAPABILITY,
                    AGENT_CHAT_PROVIDER_WARM_CAPABILITY,
                    AGENT_CHAT_PROJECTION_CAPABILITY,
                    PERMISSION_POLICY_CAPABILITY,
                    WORKSPACE_GIT_CAPABILITY,
                ]
                .map(str::to_owned)
                .to_vec(),
            ),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(negotiated) = read_frame(&mut client).await.unwrap() else {
        panic!("the daemon must negotiate");
    };
    assert_eq!(negotiated.capabilities.0.len(), 5);
    client
}

async fn exchange(client: &mut DuplexStream, request: &impl Serialize) -> Value {
    write_json_frame(client, request).await.unwrap();
    read_json_frame(client).await.unwrap()
}

fn unavailable(path: &str) -> Value {
    json!({"type": "error", "body": {
        "code": "workspaceUnavailable",
        "message": format!("the workspace folder {path} does not exist or is not a directory"),
        "workspace": {"path": path}
    }})
}

fn create(workspace_path: &str) -> AgentChatIntentFrame {
    AgentChatIntentFrame::CreateConversation {
        request_id: AgentChatRequestId("create".into()),
        receipt_id: ReceiptId(format!("create:{workspace_path}")),
        workspace_path: workspace_path.into(),
        selection: Some(AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "haiku".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        }),
    }
}

fn snapshot(conversation_id: &str) -> AgentChatProjectionFrame {
    AgentChatProjectionFrame::ConversationSnapshotRequest {
        request_id: "snapshot".into(),
        conversation_id: conversation_id.into(),
        transcript_limit: 50,
        activity_limit: 50,
    }
}

#[tokio::test]
async fn a_deleted_workspace_folder_is_one_typed_answer_and_its_history_stays_readable() {
    let data = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("project");
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.display().to_string();
    let runtime = build_runtime(
        data.path(),
        &RuntimeCapabilityProfile::new([
            RuntimeCapabilityFeature::AgentChat,
            RuntimeCapabilityFeature::AgentChatProjection,
            RuntimeCapabilityFeature::WorkspaceGit,
        ]),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut client = connect(&runtime).await;
    let created = exchange(&mut client, &create(&folder)).await;
    let conversation_id = created["body"]["conversationId"]
        .as_str()
        .unwrap()
        .to_owned();
    let run_id = created["body"]["runId"].as_str().unwrap().to_owned();
    let resolved = exchange(
        &mut client,
        &WorkspaceGitFrame::ResolveRequest {
            request_id: "resolve".into(),
            workspace_path: folder.clone(),
        },
    )
    .await;
    let workspace_id = resolved["body"]["workspaceId"].as_str().unwrap().to_owned();
    let canonical = resolved["body"]["canonicalPath"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = exchange(&mut client, &snapshot(&conversation_id)).await;
    assert_eq!(before["type"], "conversationSnapshot");

    std::fs::remove_dir(&folder).unwrap();

    let send = AgentChatIntentFrame::SendPrompt {
        request_id: AgentChatRequestId("send".into()),
        receipt_id: ReceiptId("send".into()),
        conversation_id: AgentChatConversationId(conversation_id.clone()),
        text: "hello".into(),
        attachment_ids: Vec::new(),
    };
    let warm = AgentChatIntentFrame::WarmProvider(AgentChatProviderWarm {
        request_id: AgentChatRequestId("warm".into()),
        conversation_id: AgentChatConversationId(conversation_id.clone()),
        run_id: AgentChatRunId(run_id),
    });
    let policy = PolicyRecord {
        policy_id: "policy".into(),
        workspace_id: workspace_id.clone(),
        scope: PolicyScope::ProviderPermissions,
        revision: 2,
        mode: PermissionMode::AutoAcceptEdits,
        containment: gent_types::Containment::Sandboxed,
        allowed_tools: Vec::new(),
        allowed_categories: Vec::new(),
        allowed_commands: Vec::new(),
    };
    assert_eq!(exchange(&mut client, &send).await, unavailable(&canonical));
    assert_eq!(exchange(&mut client, &warm).await, unavailable(&canonical));
    assert_eq!(
        exchange(
            &mut client,
            &WorkspaceGitFrame::StatusRequest {
                request_id: "status".into(),
                workspace_id: workspace_id.clone(),
            },
        )
        .await,
        unavailable(&canonical)
    );
    assert_eq!(
        exchange(
            &mut client,
            &PermissionPolicyFrame::Current {
                request_id: "policy".into(),
                workspace_id,
            },
        )
        .await,
        unavailable(&canonical)
    );
    assert_eq!(
        exchange(
            &mut client,
            &PermissionPolicyFrame::Save {
                request_id: "save".into(),
                policy,
                bypass_consent: false,
            },
        )
        .await,
        unavailable(&canonical)
    );
    assert_eq!(
        exchange(
            &mut client,
            &WorkspaceGitFrame::ResolveRequest {
                request_id: "resolve-again".into(),
                workspace_path: folder.clone(),
            },
        )
        .await,
        unavailable(&folder)
    );
    assert_eq!(
        exchange(&mut client, &create(&folder)).await,
        unavailable(&folder)
    );
    assert_eq!(
        exchange(&mut client, &snapshot(&conversation_id)).await,
        before
    );
}
