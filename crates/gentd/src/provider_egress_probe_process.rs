#[cfg(windows)]
use gent_drivers::lock::rechecked_identity;
use gent_types::RunVersionLock;
#[cfg(windows)]
use std::process::{Child, Command, Stdio};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::Path,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct ProbeOutput {
    pub stdout: Vec<u8>,
    pub stderr_empty: bool,
    pub success: bool,
}
pub(super) struct ProbeFailure {
    pub known: bool,
    retained: Option<Box<dyn Send>>,
}
impl ProbeFailure {
    pub fn known() -> Self {
        Self {
            known: true,
            retained: None,
        }
    }
    #[cfg(any(windows, test))]
    pub fn unknown() -> Self {
        Self {
            known: false,
            retained: None,
        }
    }
    pub fn retained(value: impl Send + 'static) -> Self {
        Self {
            known: false,
            retained: Some(Box::new(value)),
        }
    }
}
impl std::fmt::Debug for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProbeFailure")
            .field("known", &self.known)
            .field("retained", &self.retained.is_some())
            .finish()
    }
}
#[cfg(windows)]
pub(super) fn environment() -> BTreeMap<String, String> {
    [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PATH",
        "PATHEXT",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PROGRAMDATA",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok().map(|value| (name.into(), value)))
    .collect()
}
#[cfg(windows)]
pub(super) fn run(
    lock: &RunVersionLock,
    cwd: &Path,
    arguments: &[String],
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<ProbeOutput, ProbeFailure> {
    let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()
        .map_err(|_| ProbeFailure::known())?;
    run_with_helper(
        helper.image_lock().clone(),
        lock.clone(),
        cwd.to_path_buf(),
        arguments.to_vec(),
        environment.clone(),
        timeout,
    )
}
#[cfg(windows)]
#[path = "provider_egress_probe_windows.rs"]
mod windows;
#[cfg(windows)]
use windows::run_with_helper;
#[cfg(windows)]
fn await_with_authority(
    worker: JoinHandle<Result<ProbeOutput, ProbeFailure>>,
    deadline: Instant,
    authority: std::sync::Arc<Vec<File>>,
) -> Result<ProbeOutput, ProbeFailure> {
    await_worker(worker, deadline).map_err(|failure| {
        if failure.known {
            failure
        } else {
            ProbeFailure::retained((failure, authority))
        }
    })
}
#[cfg(windows)]
fn await_worker(
    worker: JoinHandle<Result<ProbeOutput, ProbeFailure>>,
    deadline: Instant,
) -> Result<ProbeOutput, ProbeFailure> {
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            return Err(ProbeFailure::retained(worker));
        }
        thread::sleep(Duration::from_millis(10));
    }
    worker.join().map_err(|_| ProbeFailure::unknown())?
}
#[cfg(windows)]
fn bounded(reader: impl Read, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.take(limit + 1).read_to_end(&mut output)?;
    if output.len() as u64 > limit {
        return Err(std::io::Error::other("credential probe output bound"));
    }
    Ok(output)
}
// None is unauthenticated/Unknown. Some(Err) is an authenticated Known
// settled failure (including read/size errors), never a successful capture.
#[cfg(windows)]
fn decode(bytes: &[u8]) -> Option<Result<ProbeOutput, ProbeFailure>> {
    if bytes.len() < 20 || &bytes[..8] != b"GENTPRB2" {
        return None;
    }
    let word = |start| u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
    let flags = word(8);
    let code = word(12);
    let length = word(16) as usize;
    if flags & !7 != 0
        || length > 65536
        || bytes.len() != 20 + length
        || ((flags & 1 != 0) != (code == 0))
    {
        return None;
    }
    if flags & 4 == 0 {
        return Some(Err(ProbeFailure::known()));
    }
    Some(Ok(ProbeOutput {
        stdout: bytes[20..].to_vec(),
        stderr_empty: flags & 2 != 0,
        success: flags & 1 != 0,
    }))
}
#[cfg(test)]
#[path = "provider_egress_probe_process_tests.rs"]
mod tests;
