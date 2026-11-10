//! Windows exact inherited-capability disposition. Never pass a path to helper.
use super::super::probe_process::ProbeFailure;
use super::{File, identity::Snapshot};
use std::{
    io::Read,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
pub(super) fn mark(file: &File, snapshot: Snapshot, deadline: Instant) -> Result<(), ProbeFailure> {
    use std::os::windows::fs::OpenOptionsExt;
    if Instant::now() >= deadline {
        return Err(ProbeFailure::known());
    }
    let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()
        .map_err(|_| ProbeFailure::known())?;
    let lock = helper.image_lock().clone();
    let image = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&lock.canonical_path)
        .map_err(|_| ProbeFailure::known())?;
    if gent_drivers::lock::rechecked_identity(&lock).ok().as_ref() != Some(&lock) {
        return Err(ProbeFailure::known());
    }
    let authority = Arc::new(image);
    let worker_authority = authority.clone();
    let input = file.try_clone().map_err(|_| ProbeFailure::known())?;
    let kind = if snapshot.directory { 2u32 } else { 1u32 };
    let worker = std::thread::Builder::new()
        .name("probe-handle-unlink".into())
        .spawn(move || {
            let mut command = gent_types::hidden_command(&lock.canonical_path);
            command
                .arg("host-unlink-probe-handle")
                .arg(snapshot.volume.to_string())
                .arg(snapshot.index.to_string())
                .arg(snapshot.bytes.to_string())
                .arg(if kind == 2 { "directory" } else { "file" });
            command.env_clear();
            for name in ["SystemRoot", "WINDIR"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            command
                .stdin(Stdio::from(input))
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut child = command.spawn().map_err(|_| ProbeFailure::known())?;
            let mut output = Vec::new();
            let read = match child.stdout.take() {
                Some(pipe) => pipe.take(21).read_to_end(&mut output),
                None => return Err(ProbeFailure::retained((child, worker_authority))),
            };
            let status = child.wait();
            let decoded = if read.is_ok() {
                decode(&output, status.ok().and_then(|v| v.code()), kind)
            } else {
                None
            };
            match decoded {
                Some(true) => Ok(()),
                Some(false) => Err(ProbeFailure::known()),
                None => Err(ProbeFailure::retained((child, worker_authority))),
            }
        })
        .map_err(|_| ProbeFailure::known())?;
    let deadline = deadline.min(Instant::now() + Duration::from_secs(2));
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            return Err(ProbeFailure::retained((worker, authority)));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    worker
        .join()
        .map_err(|_| ProbeFailure::retained(authority))?
}
fn decode(bytes: &[u8], exit: Option<i32>, kind: u32) -> Option<bool> {
    if bytes.len() != 20 || &bytes[..8] != b"GENTUNL1" {
        return None;
    }
    let word = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
    let status = word(8);
    let error = word(12);
    if word(16) != kind {
        return None;
    }
    match (exit, status, error) {
        (Some(0), 0, 0) => Some(true),
        (Some(125), 1..=6, error) if error != 0 => Some(false),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame(status: u32, error: u32, kind: u32) -> Vec<u8> {
        let mut b = b"GENTUNL1".to_vec();
        for v in [status, error, kind] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }
    #[test]
    fn disposition_acknowledgment_is_exact_and_conflicts_are_unknown() {
        assert_eq!(decode(&frame(0, 0, 1), Some(0), 1), Some(true));
        assert_eq!(decode(&frame(6, 5, 1), Some(125), 1), Some(false));
        for (bytes, code) in [
            (frame(0, 0, 1), 125),
            (frame(6, 0, 1), 125),
            (frame(0, 5, 1), 0),
            (frame(0, 0, 2), 0),
        ] {
            assert_eq!(decode(&bytes, Some(code), 1), None);
        }
        let mut extra = frame(0, 0, 1);
        extra.push(0);
        assert_eq!(decode(&extra, Some(0), 1), None);
    }
}
