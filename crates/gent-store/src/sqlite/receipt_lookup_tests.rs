use gent_types::{ReceiptId, ReceiptIntentKind, ReceiptLookup, ReceiptStatus};

use crate::SqliteLedger;

fn insert(ledger: &SqliteLedger, receipt: &str, status: &str, kind: &str) {
    ledger
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO receipts (idempotency_key, receipt_id, status, host_epoch, kind, payload_digest) VALUES (?1, ?2, ?3, 1, ?4, 'digest')",
            [format!("key-{receipt}"), receipt.into(), status.into(), kind.into()],
        )
        .unwrap();
}

fn lookup(ledger: &SqliteLedger, receipt: &str) -> ReceiptLookup {
    ledger.lookup_receipt(&ReceiptId(receipt.into())).unwrap()
}

#[test]
fn every_intent_reports_accepted_rejected_or_never_seen_without_side_effects() {
    let ledger = SqliteLedger::in_memory().unwrap();
    for (prefix, kind, intent, code) in [
        (
            "prompt",
            "agentChatPrompt",
            ReceiptIntentKind::Prompt,
            "agentChatPromptRejected",
        ),
        (
            "steer",
            "agentChatSteerQueuedPrompt",
            ReceiptIntentKind::Steer,
            "queuedPromptNotSteerable",
        ),
        (
            "turn-steer",
            "agentChatSteerQueuedPromptForTurn",
            ReceiptIntentKind::Steer,
            "queuedPromptNotSteerable",
        ),
        (
            "decision",
            "agentChatPermissionDecision",
            ReceiptIntentKind::PermissionDecision,
            "agentChatPermissionExpired",
        ),
    ] {
        let accepted = format!("{prefix}-accepted");
        let rejected = format!("{prefix}-rejected");
        let unseen = format!("{prefix}-unseen");
        insert(&ledger, &accepted, "settled", kind);
        insert(&ledger, &rejected, "rejected", kind);
        assert!(
            matches!(
                lookup(&ledger, &accepted),
                ReceiptLookup::Accepted { intent: found, receipt, prompt: None }
                    if found == intent && receipt.status == ReceiptStatus::Settled
            ),
            "{kind}"
        );
        assert_eq!(
            lookup(&ledger, &rejected),
            ReceiptLookup::Rejected {
                intent,
                receipt: gent_types::Receipt {
                    receipt_id: ReceiptId(rejected.clone()),
                    idempotency_key: format!("key-{rejected}"),
                    status: ReceiptStatus::Rejected,
                    host_epoch: gent_types::HostEpoch(1),
                },
                code: code.into(),
            },
            "{kind}"
        );
        assert_eq!(lookup(&ledger, &unseen), ReceiptLookup::NeverSeen, "{kind}");
        assert_eq!(lookup(&ledger, &unseen), ReceiptLookup::NeverSeen, "{kind}");
    }
}

#[test]
fn a_receipt_of_an_intent_without_lookup_is_refused() {
    let ledger = SqliteLedger::in_memory().unwrap();
    insert(&ledger, "switch", "settled", "agentChatSwitchSelection");
    assert!(ledger.lookup_receipt(&ReceiptId("switch".into())).is_err());
}
