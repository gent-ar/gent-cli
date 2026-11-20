use std::sync::OnceLock;

use tokio::sync::watch;

fn changes() -> &'static watch::Sender<u64> {
    static CHANGES: OnceLock<watch::Sender<u64>> = OnceLock::new();
    CHANGES.get_or_init(|| watch::channel(0).0)
}

pub(crate) fn settled() {
    changes().send_modify(|revision| *revision = revision.wrapping_add(1));
}

pub(crate) fn subscribe() -> watch::Receiver<u64> {
    changes().subscribe()
}
