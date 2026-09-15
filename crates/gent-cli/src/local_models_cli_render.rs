use std::io::{IsTerminal, Write};

use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame, LocalModelInstallState};

pub(super) fn render_download_progress(frame: &LocalModelFrame) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    let Some((line, terminal)) = download_display(frame) else {
        return;
    };
    let stderr = std::io::stderr();
    let mut output = stderr.lock();
    if terminal {
        let _ = writeln!(output, "\r{line}");
    } else {
        let _ = write!(output, "\r{line}");
        let _ = output.flush();
    }
}

pub(super) fn download_display(frame: &LocalModelFrame) -> Option<(String, bool)> {
    match frame {
        LocalModelFrame::DownloadAccepted {
            model_id,
            state:
                LocalModelInstallState::Downloading {
                    downloaded_bytes,
                    total_bytes,
                },
            ..
        }
        | LocalModelFrame::DownloadProgress {
            model_id,
            downloaded_bytes,
            total_bytes,
            ..
        } => Some((
            progress_display(model_id, *downloaded_bytes, *total_bytes),
            false,
        )),
        LocalModelFrame::DownloadAccepted {
            model_id,
            state: LocalModelInstallState::Ready { size_bytes },
            ..
        }
        | LocalModelFrame::DownloadComplete {
            model_id,
            size_bytes,
            ..
        } => Some((
            format!("Downloaded {model_id} · 100% ({})", bytes(*size_bytes)),
            true,
        )),
        LocalModelFrame::DownloadFailed {
            model_id, reason, ..
        } => Some((
            format!("Could not download {model_id} · {}", failure_label(*reason)),
            true,
        )),
        _ => None,
    }
}

pub(crate) const fn failure_label(reason: LocalModelDownloadFailure) -> &'static str {
    match reason {
        LocalModelDownloadFailure::UnknownModel => "it is not in Gent's curated model list",
        LocalModelDownloadFailure::AlreadyDownloading => "it is already downloading",
        LocalModelDownloadFailure::StorageUnavailable => "the model storage is unavailable",
        LocalModelDownloadFailure::TransportFailed => "the download connection failed",
        LocalModelDownloadFailure::VerificationFailed => "the downloaded file failed verification",
        LocalModelDownloadFailure::Cancelled => "canceled",
    }
}

pub(crate) fn progress_display(model_id: &str, downloaded_bytes: u64, total_bytes: u64) -> String {
    if total_bytes == 0 {
        return format!("Downloading {model_id} · {}", bytes(downloaded_bytes));
    }
    let percent = (u128::from(downloaded_bytes) * 100 / u128::from(total_bytes)).min(100);
    format!(
        "Downloading {model_id} · {percent}% ({} / {})",
        bytes(downloaded_bytes),
        bytes(total_bytes)
    )
}

pub(super) fn bytes(value: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if value >= GIB {
        scaled_bytes(value, GIB, "GiB")
    } else if value >= MIB {
        scaled_bytes(value, MIB, "MiB")
    } else {
        format!("{value} B")
    }
}

fn scaled_bytes(value: u64, unit: u64, label: &str) -> String {
    let mut whole = value / unit;
    let mut tenths = ((value % unit) * 10 + unit / 2) / unit;
    if tenths == 10 {
        whole += 1;
        tenths = 0;
    }
    format!("{whole}.{tenths} {label}")
}
