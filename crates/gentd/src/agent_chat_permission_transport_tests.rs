use async_trait::async_trait;
use gent_protocol::{AgentChatPermissionFrame, read_json_frame};
use gent_types::AgentChatRequestId;
use serde_json::json;
use tokio::io::duplex;

use crate::{
    agent_chat_permission_api::AgentChatPermissionPort,
    agent_chat_permission_transport::dispatch_port,
};

#[derive(Clone)]
struct Port;

#[async_trait]
impl AgentChatPermissionPort for Port {
    async fn exchange(
        &self,
        frame: AgentChatPermissionFrame,
    ) -> Result<AgentChatPermissionFrame, String> {
        let AgentChatPermissionFrame::PendingRead { request_id, .. } = frame else {
            return Err("unexpected".into());
        };
        Ok(AgentChatPermissionFrame::Pending {
            request_id,
            request: None,
        })
    }
}

#[tokio::test]
async fn pending_read_is_dispatched_only_as_a_typed_permission_frame() {
    let (mut reader, mut writer) = duplex(2048);
    let request = json!({"type":"pendingRead","body":{
        "requestId":"request-1","conversationId":"conversation-1","runId":"run-1"
    }});
    assert!(dispatch_port(&mut writer, &Port, &request).await.unwrap());
    assert!(matches!(
        read_json_frame::<_, AgentChatPermissionFrame>(&mut reader).await.unwrap(),
        AgentChatPermissionFrame::Pending { request_id, request: None } if request_id.0 == "request-1"
    ));
}

#[tokio::test]
async fn server_only_permission_frames_are_rejected_before_the_port() {
    let (mut reader, mut writer) = duplex(2048);
    let response = AgentChatPermissionFrame::Pending {
        request_id: AgentChatRequestId("request-1".into()),
        request: None,
    };
    assert!(
        dispatch_port(&mut writer, &Port, &serde_json::to_value(response).unwrap())
            .await
            .unwrap()
    );
    let frame: gent_protocol::WireFrame = gent_protocol::read_frame(&mut reader).await.unwrap();
    assert!(
        matches!(frame, gent_protocol::WireFrame::Error { code, .. } if code == "invalidAgentChatPermission")
    );
}
