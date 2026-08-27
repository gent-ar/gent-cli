use gent_core::Run;
use gent_ports::{IngressMode, Ledger, LedgerError, WorktreeLease};
use gent_runtime::{Coordinator, RuntimeError};
use gent_store::SqliteLedger;
use gent_types::{CapabilitySet, Command, HostEpoch, ReceiptId, ReceiptStatus};
use serde_json::json;

fn command(key: &str, epoch: u64, kind: &str) -> Command {
    Command {
        receipt_id: ReceiptId::new(),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(epoch),
        kind: kind.into(),
        payload: json!({ "example": true }),
    }
}

#[test]
fn acceptance_and_terminal_events_are_idempotent() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
    let command = command("once", 1, "ping");
    let first = coordinator.submit(&command).unwrap();
    assert_eq!(first, coordinator.submit(&command).unwrap());
    assert_eq!(ledger.read_event_page(0, 100).unwrap().events.len(), 2);
}

#[test]
fn a_closed_or_fenced_host_cannot_accept_commands() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    assert_eq!(
        coordinator.close_ingress(HostEpoch(1)).unwrap().mode,
        IngressMode::Closed
    );
    assert!(matches!(
        coordinator.submit(&command("closed", 1, "ping")),
        Err(RuntimeError::Ledger(LedgerError::IngressClosed { .. }))
    ));
    assert_eq!(
        coordinator.fence_and_open(HostEpoch(1)).unwrap().epoch,
        HostEpoch(2)
    );
    assert!(matches!(
        coordinator.submit(&command("old", 1, "ping")),
        Err(RuntimeError::Ledger(LedgerError::StaleEpoch { .. }))
    ));
}

#[test]
fn decisions_settle_without_provider_acknowledgement() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    assert_eq!(
        coordinator
            .submit(&command("decision", 1, "decision"))
            .unwrap()
            .status,
        ReceiptStatus::Unprovable
    );
}

#[test]
fn provider_switches_and_stale_leases_are_durable() {
    let coordinator =
        Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
    let root = Run {
        id: "root".into(),
        parent_run_id: None,
        provider: "claude".into(),
    };
    coordinator.create_run(&root).unwrap();
    let child = coordinator
        .switch_provider(&root, "child".into(), "codex".into())
        .unwrap();
    let first = WorktreeLease {
        worktree_id: "tree".into(),
        run_id: root.id,
        lease_token: "one".into(),
        host_epoch: HostEpoch(1),
    };
    assert!(matches!(
        coordinator.claim_worktree_lease(&first),
        Ok(gent_ports::LeaseClaim::Acquired(_))
    ));
    coordinator.close_ingress(HostEpoch(1)).unwrap();
    coordinator.fence_and_open(HostEpoch(1)).unwrap();
    let second = WorktreeLease {
        worktree_id: "tree".into(),
        run_id: child.id,
        lease_token: "two".into(),
        host_epoch: HostEpoch(2),
    };
    assert!(matches!(
        coordinator.claim_worktree_lease(&second),
        Ok(gent_ports::LeaseClaim::Recovered { .. })
    ));
}
