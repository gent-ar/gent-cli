use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROVIDER_WARM_CAPABILITY, AgentChatIntentFrame,
    ProviderWarmOutcome, WireFrame, read_frame, read_json_frame,
};
use gent_types::{AgentChatRunId, CapabilitySet};
use serde_json::json;
use tokio::io::duplex;

use crate::agent_chat_intent_error::AgentChatIntentError;
use crate::agent_chat_transport::{IntentPort, dispatch_port};

struct WarmPort {
    run_id: &'static str,
}

impl IntentPort for WarmPort {
    fn exchange(
        &self,
        request: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let AgentChatIntentFrame::WarmProvider(warm) = request else {
            return Err("unexpected intent".into());
        };
        let mut result = warm.result(ProviderWarmOutcome::Warming);
        result.run_id = AgentChatRunId(self.run_id.into());
        Ok(vec![AgentChatIntentFrame::ProviderWarm(result)])
    }
}

fn warm() -> serde_json::Value {
    json!({
        "type": "warmProvider",
        "body": {"requestId": "request-1", "conversationId": "conversation-1", "runId": "run-1"}
    })
}

fn negotiated(warm: bool) -> CapabilitySet {
    let mut capabilities = vec![AGENT_CHAT_INTENTS_CAPABILITY.to_owned()];
    if warm {
        capabilities.push(AGENT_CHAT_PROVIDER_WARM_CAPABILITY.into());
    }
    CapabilitySet(capabilities)
}

#[tokio::test]
async fn a_warm_up_is_answered_with_its_correlated_outcome() {
    let (mut reader, mut writer) = duplex(4096);
    let port = WarmPort { run_id: "run-1" };

    assert!(
        dispatch_port(&mut writer, &port, &negotiated(true), &warm())
            .await
            .unwrap()
    );

    let reply = read_json_frame::<_, AgentChatIntentFrame>(&mut reader)
        .await
        .unwrap();
    assert!(matches!(
        reply,
        AgentChatIntentFrame::ProviderWarm(result)
            if result.request_id.0 == "request-1"
                && result.outcome == ProviderWarmOutcome::Warming
    ));
}

#[tokio::test]
async fn a_warm_up_is_unhandled_without_its_capability() {
    let (_reader, mut writer) = duplex(4096);
    let port = WarmPort { run_id: "run-1" };

    assert!(
        !dispatch_port(&mut writer, &port, &negotiated(false), &warm())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_warm_result_for_another_run_is_rejected() {
    let (mut reader, mut writer) = duplex(4096);
    let port = WarmPort { run_id: "run-2" };

    assert!(
        dispatch_port(&mut writer, &port, &negotiated(true), &warm())
            .await
            .unwrap()
    );

    let reply: WireFrame = read_frame(&mut reader).await.unwrap();
    assert!(matches!(reply, WireFrame::Error { code, .. } if code == "invalidAgentChatResponse"));
}
