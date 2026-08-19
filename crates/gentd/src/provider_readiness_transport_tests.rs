use gent_protocol::{
    PROVIDER_READINESS_CAPABILITY, ProviderReadinessFrame, ProviderReadinessUnavailable, WireFrame,
    read_frame, read_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRunId, CapabilitySet};
use serde_json::json;
use tokio::io::duplex;

use super::{ReadinessPort, dispatch_port};

#[derive(Clone)]
struct Port {
    reply: ProviderReadinessFrame,
}

impl ReadinessPort for Port {
    fn assess(&self, _: ProviderReadinessFrame) -> Result<ProviderReadinessFrame, String> {
        Ok(self.reply.clone())
    }
}

fn request() -> serde_json::Value {
    json!({ "type": "assess", "body": { "conversationId": "c", "runId": "r" } })
}

fn capabilities() -> CapabilitySet {
    CapabilitySet(vec![PROVIDER_READINESS_CAPABILITY.into()])
}

fn unavailable(conversation_id: &str, run_id: &str) -> ProviderReadinessFrame {
    ProviderReadinessFrame::Unavailable {
        conversation_id: AgentChatConversationId(conversation_id.into()),
        run_id: AgentChatRunId(run_id.into()),
        reason: ProviderReadinessUnavailable::ProvenanceUnreadable,
    }
}

#[tokio::test]
async fn transport_requires_capability_before_parsing_or_dispatching() {
    let (_, mut writer) = duplex(1024);
    assert!(
        !dispatch_port(
            &mut writer,
            &Port {
                reply: unavailable("c", "r")
            },
            &CapabilitySet::default(),
            &request(),
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn response_input_is_rejected_before_reaching_the_port() {
    let (mut reader, mut writer) = duplex(1024);
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: unavailable("c", "r")
            },
            &capabilities(),
            &serde_json::to_value(unavailable("c", "r")).unwrap(),
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidProviderReadiness")
    );
}

#[tokio::test]
async fn readiness_reply_must_retain_exact_conversation_and_run() {
    let (mut reader, mut writer) = duplex(1024);
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: unavailable("other", "r")
            },
            &capabilities(),
            &request(),
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidProviderReadiness")
    );
    let (mut reader, mut writer) = duplex(1024);
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: unavailable("c", "r")
            },
            &capabilities(),
            &request(),
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_, ProviderReadinessFrame>(&mut reader).await.unwrap(), ProviderReadinessFrame::Unavailable { conversation_id, run_id, .. } if conversation_id.0 == "c" && run_id.0 == "r")
    );
}
