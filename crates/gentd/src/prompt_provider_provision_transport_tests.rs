use gent_protocol::{
    PROMPT_PROVIDER_PROVISION_CAPABILITY, PromptProviderProvisionFrame,
    PromptProviderProvisionState, WireFrame, read_frame, read_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, CapabilitySet, HostEpoch, Receipt, ReceiptId,
    ReceiptStatus,
};
use serde_json::json;
use tokio::io::duplex;

use super::{PromptProviderProvisionPort, dispatch_port};

#[derive(Clone)]
struct Port {
    reply: PromptProviderProvisionFrame,
}

impl PromptProviderProvisionPort for Port {
    fn confirm(
        &self,
        _: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        Ok(self.reply.clone())
    }
}

fn request() -> serde_json::Value {
    json!({ "type": "confirm", "body": {
        "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
        "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
        "consentGranted": true, "reviewedPlanDigest": "a".repeat(64)
    } })
}

fn capabilities() -> CapabilitySet {
    CapabilitySet(vec![PROMPT_PROVIDER_PROVISION_CAPABILITY.into()])
}

fn result(receipt_id: &str, conversation_id: &str) -> PromptProviderProvisionFrame {
    PromptProviderProvisionFrame::Result {
        receipt: Receipt {
            receipt_id: ReceiptId(receipt_id.into()),
            idempotency_key: "key".into(),
            status: ReceiptStatus::Settled,
            host_epoch: HostEpoch(1),
        },
        prompt_receipt_id: ReceiptId("prompt".into()),
        conversation_id: AgentChatConversationId(conversation_id.into()),
        run_id: AgentChatRunId("run".into()),
        state: PromptProviderProvisionState::Completed,
    }
}

#[tokio::test]
async fn transport_requires_negotiated_capability_before_parsing_or_dispatching() {
    let (_, mut writer) = duplex(1024);
    assert!(
        !dispatch_port(
            &mut writer,
            &Port {
                reply: result("receipt", "conversation")
            },
            &CapabilitySet::default(),
            &request(),
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn results_and_invalid_confirmations_never_reach_the_port() {
    let (mut reader, mut writer) = duplex(1024);
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: result("receipt", "conversation")
            },
            &capabilities(),
            &serde_json::to_value(result("receipt", "conversation")).unwrap(),
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        read_frame(&mut reader).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidPromptProviderProvision"
    ));
    let (mut reader, mut writer) = duplex(1024);
    let mut invalid = request();
    invalid["body"]["reviewedPlanDigest"] = json!("A".repeat(64));
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: result("receipt", "conversation")
            },
            &capabilities(),
            &invalid,
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        read_frame(&mut reader).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidPromptProviderProvision"
    ));
}

#[tokio::test]
async fn reply_requires_the_full_confirm_receipt_and_prompt_scope() {
    for reply in mismatched_replies() {
        let (mut reader, mut writer) = duplex(1024);
        assert!(
            dispatch_port(&mut writer, &Port { reply }, &capabilities(), &request(),)
                .await
                .unwrap()
        );
        assert!(matches!(
            read_frame(&mut reader).await.unwrap(),
            WireFrame::Error { code, .. } if code == "invalidPromptProviderProvision"
        ));
    }
    let (mut reader, mut writer) = duplex(1024);
    assert!(
        dispatch_port(
            &mut writer,
            &Port {
                reply: result("receipt", "conversation")
            },
            &capabilities(),
            &request(),
        )
        .await
        .unwrap()
    );
    assert!(matches!(
        read_json_frame::<_, PromptProviderProvisionFrame>(&mut reader)
            .await
            .unwrap(),
        PromptProviderProvisionFrame::Result { receipt, conversation_id, .. }
            if receipt.receipt_id.0 == "receipt" && conversation_id.0 == "conversation"
    ));
}

fn mismatched_replies() -> Vec<PromptProviderProvisionFrame> {
    let mut receipt_id = result("receipt", "conversation");
    receipt(&mut receipt_id).receipt_id = ReceiptId("other".into());
    let mut idempotency = result("receipt", "conversation");
    receipt(&mut idempotency).idempotency_key = "other".into();
    let mut epoch = result("receipt", "conversation");
    receipt(&mut epoch).host_epoch = HostEpoch(2);
    let mut prompt = result("receipt", "conversation");
    if let PromptProviderProvisionFrame::Result {
        prompt_receipt_id, ..
    } = &mut prompt
    {
        *prompt_receipt_id = ReceiptId("other".into());
    }
    let conversation = result("receipt", "other");
    let mut run = result("receipt", "conversation");
    if let PromptProviderProvisionFrame::Result { run_id, .. } = &mut run {
        *run_id = AgentChatRunId("other".into());
    }
    vec![receipt_id, idempotency, epoch, prompt, conversation, run]
}

fn receipt(frame: &mut PromptProviderProvisionFrame) -> &mut Receipt {
    let PromptProviderProvisionFrame::Result { receipt, .. } = frame else {
        unreachable!();
    };
    receipt
}
