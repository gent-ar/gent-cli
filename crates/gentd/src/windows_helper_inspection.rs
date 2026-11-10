//! A pinned helper is the only OS-query authority; no unsafe daemon FFI.
use crate::windows_sandbox_runtime::PinnedSandboxHelper;
use std::{
    fs::OpenOptions,
    io::Read,
    os::windows::fs::OpenOptionsExt,
    process::{Child, Stdio},
    time::{Duration, Instant},
};

struct OwnedInspector(Child);
impl Drop for OwnedInspector {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn inspect<T: serde::de::DeserializeOwned>(
    helper: &PinnedSandboxHelper,
    mode: &str,
    arguments: &[String],
) -> Result<T, String> {
    let lock = helper.image_lock();
    let _image = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&lock.canonical_path)
        .map_err(|_| "cannot lock reviewed inspector image")?;
    if gent_drivers::lock::rechecked_identity(lock).map_err(|_| "cannot recheck inspector image")?
        != *lock
    {
        return Err("reviewed inspector image changed".into());
    }
    let mut child = OwnedInspector(
        gent_types::hidden_command(&lock.canonical_path)
            .arg(mode)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "reviewed OS inspector unavailable")?,
    );
    let output = child
        .0
        .stdout
        .take()
        .ok_or("OS inspector output unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.take(262_145).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.0.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.0.kill();
                let _ = child.0.wait();
                break Err("OS inspector timed out or exited unexpectedly");
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "OS inspector reader failed")?
        .map_err(|_| "OS inspector output failed")?;
    if !status?.success() || bytes.len() > 262_144 {
        return Err("OS inspector rejected provenance or exceeded output limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "OS inspector returned an invalid report".into())
}
