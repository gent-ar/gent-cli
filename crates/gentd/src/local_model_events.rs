use gent_ports::Ledger;
use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame};
use gent_store::SqliteLedger;
use gent_types::{Event, HostEpoch, ReceiptId};

pub(crate) fn publish(
    ledger: &SqliteLedger,
    host_epoch: HostEpoch,
    frame: LocalModelFrame,
) -> Result<(), String> {
    match frame {
        LocalModelFrame::DownloadAccepted { .. }
        | LocalModelFrame::DownloadProgress { .. }
        | LocalModelFrame::DownloadComplete { .. }
        | LocalModelFrame::DownloadFailed { .. } => {}
        _ => return Err("local-model events must describe download state".into()),
    }
    ledger
        .append_event(&Event {
            cursor: 0,
            event_id: uuid::Uuid::new_v4().to_string(),
            receipt_id: ReceiptId::new(),
            host_epoch,
            kind: "localModelDownload".into(),
            payload: serde_json::to_value(frame).map_err(|error| error.to_string())?,
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn failure_for(message: &str) -> LocalModelDownloadFailure {
    if message.contains("not in Gent's curated catalog") || message.contains("unknown curated") {
        LocalModelDownloadFailure::UnknownModel
    } else if message.contains("already active") {
        LocalModelDownloadFailure::AlreadyDownloading
    } else if message.contains("storage")
        || message.contains("I/O")
        || message.contains("destination")
        || message.contains("not a regular file")
        || message.contains("could not inspect")
    {
        LocalModelDownloadFailure::StorageUnavailable
    } else if message.contains("size")
        || message.contains("exceeded")
        || message.contains("partial")
        || message.contains("digest")
        || message.contains("does not match")
    {
        LocalModelDownloadFailure::VerificationFailed
    } else {
        LocalModelDownloadFailure::TransportFailed
    }
}

pub(crate) const fn failure_text(reason: LocalModelDownloadFailure) -> &'static str {
    match reason {
        LocalModelDownloadFailure::UnknownModel => "unknown model",
        LocalModelDownloadFailure::AlreadyDownloading => "already downloading",
        LocalModelDownloadFailure::StorageUnavailable => "storage unavailable",
        LocalModelDownloadFailure::TransportFailed => "transport failed",
        LocalModelDownloadFailure::VerificationFailed => "verification failed",
        LocalModelDownloadFailure::Cancelled => "download canceled",
    }
}

#[cfg(test)]
mod tests {
    use gent_protocol::LocalModelDownloadFailure;

    use super::failure_for;

    #[test]
    fn classifies_integrity_and_storage_failures_before_transport() {
        assert_eq!(
            failure_for("download does not match the curated SHA-256"),
            LocalModelDownloadFailure::VerificationFailed
        );
        assert_eq!(
            failure_for("local model file is not a regular file"),
            LocalModelDownloadFailure::StorageUnavailable
        );
        assert_eq!(
            failure_for("download transport failed: connection reset"),
            LocalModelDownloadFailure::TransportFailed
        );
    }
}
