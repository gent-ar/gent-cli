use gent_ports::Ledger;
use gent_types::HostEpoch;

use crate::SqliteLedger;

#[test]
fn a_committed_write_notifies_and_a_read_does_not() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut commits = ledger.commits();
    ledger.host_ingress().unwrap();
    assert!(!commits.has_changed().unwrap());
    ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(commits.has_changed().unwrap());
    commits.borrow_and_update();
    ledger.host_ingress().unwrap();
    assert!(!commits.has_changed().unwrap());
}

#[test]
fn the_notification_lands_after_the_write_is_readable() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut commits = ledger.commits();
    let reader = ledger.clone();
    let observed = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(commits.changed())
            .unwrap();
        reader.host_ingress().unwrap()
    });
    let written = ledger.close_ingress(HostEpoch(1)).unwrap();
    assert_eq!(observed.join().unwrap(), written);
}

#[test]
fn a_blocking_wait_wakes_on_a_commit_and_times_out_without_one() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let mut seen = ledger.commit_count();
    let soon = std::time::Instant::now() + std::time::Duration::from_millis(50);
    assert!(!ledger.wait_for_commit(&mut seen, soon));
    let writer = ledger.clone();
    let written = std::thread::spawn(move || writer.close_ingress(HostEpoch(1)).unwrap());
    let later = std::time::Instant::now() + std::time::Duration::from_secs(30);
    assert!(ledger.wait_for_commit(&mut seen, later));
    assert_eq!(seen, ledger.commit_count());
    assert_eq!(ledger.host_ingress().unwrap(), written.join().unwrap());
}
