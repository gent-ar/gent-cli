use std::{
    io::Read,
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};

pub(super) fn capture_acp_stderr(
    stderr: &mut std::process::ChildStderr,
    capture: Arc<Mutex<Vec<u8>>>,
) {
    const MAX_STDERR_BYTES: usize = 4 * 1024;
    let mut chunk = [0_u8; 512];
    loop {
        let Ok(read) = stderr.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        let Ok(mut captured) = capture.lock() else {
            return;
        };
        let remaining = MAX_STDERR_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&chunk[..read.min(remaining)]);
    }
}

pub(super) fn acp_exit_error(
    status: std::process::ExitStatus,
    stderr: &Arc<Mutex<Vec<u8>>>,
) -> String {
    let captured = stderr
        .lock()
        .map_or_else(|_| Vec::new(), |bytes| bytes.clone());
    format!(
        "Claurst ACP exited before producing a response (status: {status}; stderr bytes: {}; stderr sha256: {:x})",
        captured.len(),
        Sha256::digest(&captured)
    )
}
