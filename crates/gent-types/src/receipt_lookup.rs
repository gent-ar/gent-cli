use serde::{Deserialize, Serialize};

use crate::{AgentChatPromptDelivery, Receipt, ReceiptStatus};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptIntentKind {
    Prompt,
    Steer,
    PermissionDecision,
}

impl ReceiptIntentKind {
    #[must_use]
    pub fn from_command_kind(kind: &str) -> Option<Self> {
        match kind {
            "agentChatPrompt" => Some(Self::Prompt),
            "agentChatSteerPromptForTurn"
            | "agentChatSteerQueuedPrompt"
            | "agentChatSteerQueuedPromptForTurn" => Some(Self::Steer),
            "agentChatPermissionDecision" => Some(Self::PermissionDecision),
            _ => None,
        }
    }

    #[must_use]
    pub const fn rejection_code(self) -> &'static str {
        match self {
            Self::Prompt => "agentChatPromptRejected",
            Self::Steer => "queuedPromptNotSteerable",
            Self::PermissionDecision => "agentChatPermissionExpired",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptPromptOutcome {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub delivery: AgentChatPromptDelivery,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ReceiptLookup {
    Accepted {
        intent: ReceiptIntentKind,
        receipt: Receipt,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<ReceiptPromptOutcome>,
    },
    Rejected {
        intent: ReceiptIntentKind,
        receipt: Receipt,
        code: String,
    },
    NeverSeen,
}

impl ReceiptLookup {
    #[must_use]
    pub fn found(
        intent: ReceiptIntentKind,
        receipt: Receipt,
        prompt: Option<ReceiptPromptOutcome>,
    ) -> Self {
        if receipt.status == ReceiptStatus::Rejected {
            Self::Rejected {
                intent,
                receipt,
                code: intent.rejection_code().into(),
            }
        } else {
            Self::Accepted {
                intent,
                receipt,
                prompt,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReceiptIntentKind, ReceiptLookup};
    use crate::{HostEpoch, Receipt, ReceiptId, ReceiptStatus};

    fn receipt(status: ReceiptStatus) -> Receipt {
        Receipt {
            receipt_id: ReceiptId("r".into()),
            idempotency_key: "k".into(),
            status,
            host_epoch: HostEpoch(1),
        }
    }

    #[test]
    fn a_rejected_receipt_carries_its_intent_typed_code() {
        let lookup = ReceiptLookup::found(
            ReceiptIntentKind::PermissionDecision,
            receipt(ReceiptStatus::Rejected),
            None,
        );
        let value = serde_json::to_value(&lookup).unwrap();
        assert_eq!(value["state"], "rejected");
        assert_eq!(value["intent"], "permissionDecision");
        assert_eq!(value["code"], "agentChatPermissionExpired");
        assert_eq!(
            serde_json::to_value(ReceiptLookup::NeverSeen).unwrap(),
            serde_json::json!({"state": "neverSeen"})
        );
        assert!(matches!(
            ReceiptLookup::found(
                ReceiptIntentKind::Steer,
                receipt(ReceiptStatus::Settled),
                None
            ),
            ReceiptLookup::Accepted { .. }
        ));
        assert_eq!(
            ReceiptIntentKind::from_command_kind("agentChatSwitchSelection"),
            None
        );
    }
}
