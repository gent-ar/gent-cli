use std::{io::Read, process::ChildStdout, sync::mpsc::SyncSender};

use gent_drivers::ndjson::NdjsonFramer;

pub(super) fn relay_acp_frames(stdout: ChildStdout, sender: SyncSender<Result<Vec<u8>, String>>) {
    let mut reader = stdout;
    let mut framer = NdjsonFramer::new(gent_drivers::MAX_PROVIDER_FRAME_BYTES)
        .expect("the provider frame ceiling is non-zero");
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        };
        for &byte in &chunk[..read] {
            let frame = framer.push_byte(byte);
            let notice = (framer.take_skipped_frames() > 0).then(oversized_frame_notice);
            for frame in notice.into_iter().chain(frame) {
                if sender.send(Ok(frame)).is_err() {
                    return;
                }
            }
        }
    }
}

fn oversized_frame_notice() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": crate::claurst_acp_transport::OVERSIZED_FRAME_METHOD,
    }))
    .expect("a fixed notice serializes")
}

pub(super) fn bounded_frame(frame: Vec<u8>, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    (frame.len() <= maximum_bytes)
        .then_some(frame)
        .ok_or_else(|| "Claurst ACP frame exceeds the fixed bound".into())
}

#[cfg(test)]
#[path = "claurst_local_runtime_owner_process_io_tests.rs"]
mod tests;
