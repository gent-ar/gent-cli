use gent_ports::CapabilityCatalogLedger;
use gent_store::SqliteLedger;
use gent_types::{CapabilityCatalogRecord, CapabilitySet};

#[test]
fn catalog_snapshot_survives_restart_and_replaces_as_a_whole() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gent.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    ledger
        .save_capability_catalog(&CapabilityCatalogRecord {
            schema_version: 1,
            capabilities: CapabilitySet(vec!["events".into()]),
        })
        .unwrap();
    ledger
        .save_capability_catalog(&CapabilityCatalogRecord {
            schema_version: 2,
            capabilities: CapabilitySet(vec!["events".into(), "receipts".into()]),
        })
        .unwrap();
    drop(ledger);
    assert_eq!(
        SqliteLedger::open(&path)
            .unwrap()
            .capability_catalog()
            .unwrap(),
        Some(CapabilityCatalogRecord {
            schema_version: 2,
            capabilities: CapabilitySet(vec!["events".into(), "receipts".into()])
        })
    );
}
