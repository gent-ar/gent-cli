use std::{
    io::{BufRead, BufReader},
    process::ChildStdout,
    sync::mpsc::SyncSender,
};

pub(super) fn relay_acp_frames(stdout: ChildStdout, sender: SyncSender<Result<Vec<u8>, String>>) {
    const MAX_RELAY_FRAME_BYTES: usize = 256 * 1024 + 1;
    let mut reader = BufReader::new(stdout);
    loop {
        let mut frame = Vec::new();
        match reader.read_until(b'\n', &mut frame) {
            Ok(0) => return,
            Ok(_) if frame.len() > MAX_RELAY_FRAME_BYTES => {
                let _ = sender.send(Err("Claurst ACP frame exceeds the fixed bound".into()));
                return;
            }
            Ok(_) => {
                if frame.last() == Some(&b'\n') {
                    frame.pop();
                }
                if frame.last() == Some(&b'\r') {
                    frame.pop();
                }
                if sender.send(Ok(frame)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        }
    }
}

pub(super) fn bounded_frame(frame: Vec<u8>, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    (frame.len() <= maximum_bytes)
        .then_some(frame)
        .ok_or_else(|| "Claurst ACP frame exceeds the fixed bound".into())
}
