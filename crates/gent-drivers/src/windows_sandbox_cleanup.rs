use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{SandboxNetworkPeer, lock::rechecked_identity};
use gent_types::{RunVersionLock, SandboxedLaunchRequest};

#[derive(Debug)]
pub(super) struct CleanupContext {
    helper: RunVersionLock,
    state_root: Option<PathBuf>,
    peer_name: Option<String>,
}

impl CleanupContext {
    pub(super) fn new(request: &SandboxedLaunchRequest, helper: RunVersionLock) -> Self {
        Self {
            helper,
            state_root: request
                .profile
                .private_state()
                .map(|state| state.root().to_path_buf()),
            peer_name: request
                .profile
                .gateway()
                .map(|gateway| format!("GentGateway-{}", gateway.peer_nonce())),
        }
    }

    pub(super) fn cleanup(&self, peer: Option<&SandboxNetworkPeer>) {
        // External installed package ownership belongs to the retained native adapter.
        // Never feed its PFN into the historical per-run profile-deletion interface.
        if peer.is_some_and(|peer| peer.package_full_name().is_some()) {
            return;
        }
        let (Some(root), Some(identity)) = (&self.state_root, &self.peer_name) else {
            return;
        };
        if peer.is_none() && !root.join(identity).join(".ownership").is_file() {
            return;
        }
        if rechecked_identity(&self.helper).ok().as_ref() != Some(&self.helper) {
            eprintln!("sandbox peer cleanup helper identity changed");
            return;
        }
        let mut command = gent_types::hidden_command(&self.helper.canonical_path);
        command
            .arg(if peer.is_some() {
                "cleanup-peer"
            } else {
                "cleanup-pending"
            })
            .arg(root)
            .arg(identity)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(peer) = peer {
            if peer.identity() != identity {
                return;
            }
            command
                .arg(peer.process_id().to_string())
                .arg(peer.creation_time().to_string());
        }
        if let Some(windows) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", windows);
        }
        let Ok(mut child) = command.spawn() else {
            eprintln!("sandbox peer cleanup could not start");
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = stop(&mut child);
                    eprintln!("sandbox peer cleanup was not confirmed");
                    return;
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct PendingHelper<'a> {
    child: Option<Child>,
    cleanup: &'a CleanupContext,
    pin: Option<std::fs::File>,
}

impl<'a> PendingHelper<'a> {
    pub(super) fn new(child: Child, cleanup: &'a CleanupContext, pin: std::fs::File) -> Self {
        Self {
            child: Some(child),
            cleanup,
            pin: Some(pin),
        }
    }
    pub(super) fn finish(mut self) -> Child {
        self.child.take().expect("owned pending helper")
    }

    pub(super) fn failure(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        stop(child);
        let mut text = String::new();
        child
            .stderr
            .take()?
            .take(4096)
            .read_to_string(&mut text)
            .ok()?;
        text.lines().find_map(|line| {
            let detail = line.strip_prefix("GENT-SANDBOX-ERROR ")?;
            let (label, code) = detail.split_once(':')?;
            (!label.is_empty()
                && label.len() <= 64
                && label.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                && !code.is_empty()
                && code.len() <= 10
                && code.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("native sandbox rejected launch ({label}:{code})"))
        })
    }
}
impl Deref for PendingHelper<'_> {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().expect("owned pending helper")
    }
}
impl DerefMut for PendingHelper<'_> {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("owned pending helper")
    }
}
impl Drop for PendingHelper<'_> {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            if stop(child) {
                self.cleanup.cleanup(None);
            } else if let (Some(child), Some(pin)) = (self.child.take(), self.pin.take()) {
                quarantine_pending(child, pin);
            }
        }
    }
}

fn stop(child: &mut Child) -> bool {
    if let Ok(Some(status)) = child.try_wait() {
        return matches!(status.code(), Some(0 | 125));
    }
    // EOF asks the pinned helper to stop its worker and settle both native Jobs.
    child.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(18);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return matches!(status.code(), Some(0 | 125)),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return false,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

// Deliberately process-resident: no automatic retry, cleanup or release of unknown ownership.
fn quarantine_pending(child: Child, pin: std::fs::File) {
    static PENDING: std::sync::OnceLock<std::sync::Mutex<Vec<(Child, std::fs::File)>>> =
        std::sync::OnceLock::new();
    PENDING
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((child, pin));
    eprintln!("native sandbox pending scope remains unprovable; ownership retained");
}
pub(super) fn quarantine_process(process: crate::SystemProcess, pin: std::fs::File) {
    static ACTIVE: std::sync::OnceLock<
        std::sync::Mutex<Vec<(crate::SystemProcess, std::fs::File)>>,
    > = std::sync::OnceLock::new();
    ACTIVE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((process, pin));
    eprintln!("native sandbox active scope remains unprovable; ownership retained");
}

#[cfg(test)]
#[path = "windows_sandbox_cleanup_tests.rs"]
mod tests;
