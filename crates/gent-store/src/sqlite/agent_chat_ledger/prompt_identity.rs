use gent_types::{AgentChatPromptCreate, AgentChatPromptDisposition, Command};
use serde_json::json;
use sha2::{Digest, Sha256};

pub(super) fn command_for(prompt: &AgentChatPromptCreate) -> Command {
    Command {
        receipt_id: prompt.receipt_id.clone(),
        idempotency_key: idempotency_key(prompt),
        host_epoch: prompt.host_epoch,
        kind: "agentChatPrompt".into(),
        payload: json!({ "requestId": prompt.request_id, "conversationId": prompt.conversation_id, "disposition": prompt.disposition, "textDigestSha256": digest(&prompt.text), "textByteLen": prompt.text.len(), "attachmentIds": prompt.attachment_ids }),
    }
}

pub(super) fn idempotency_key(prompt: &AgentChatPromptCreate) -> String {
    stable_identity("receipt", &prompt.request_id.0)
}

pub(super) fn stable_identity(kind: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-prompt-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("agent-chat-{kind}-{:x}", digest.finalize())
}

pub(super) fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) const fn disposition(value: AgentChatPromptDisposition) -> &'static str {
    match value {
        AgentChatPromptDisposition::Send => "send",
        AgentChatPromptDisposition::Queue => "queue",
    }
}
