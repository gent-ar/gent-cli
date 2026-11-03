use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gent_types::{
    RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxNetworkPolicy,
    SandboxedLaunchRequest,
};

use crate::SystemProcess;
use crate::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use crate::lock::rechecked_identity;
use crate::sandboxed_launch::{
    ProcessSandboxEvidence, SandboxedProviderLaunch, SandboxedProviderLaunchError as Error,
};
use crate::supervisor::{ProviderLaunch, ProviderProcess};

#[derive(Clone, Debug)]
pub struct WindowsSandboxLauncher {
    helper_lock: RunVersionLock,
    output_limit: usize,
}

impl WindowsSandboxLauncher {
    #[must_use]
    pub fn new(helper_lock: RunVersionLock, output_limit: usize) -> Self {
        Self {
            helper_lock,
            output_limit,
        }
    }
}

#[derive(Debug)]
pub struct WindowsSandboxProcess {
    process: Option<SystemProcess>,
    forced: std::sync::atomic::AtomicBool,
    stop_lock: std::sync::Mutex<()>,
    evidence: ProcessSandboxEvidence,
    helper_image: Option<std::fs::File>,
    cleanup: cleanup::CleanupContext,
}

impl SandboxedProviderLaunch for WindowsSandboxLauncher {
    type Process = WindowsSandboxProcess;

    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
    ) -> Result<Self::Process, Error> {
        use std::os::windows::fs::OpenOptionsExt;
        let run = validated_run(request, launch)?;
        let helper_image = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&self.helper_lock.canonical_path)
            .map_err(|_| Error::Unavailable)?;
        let helper = rechecked_identity(&self.helper_lock).map_err(|_| Error::LockChanged)?;
        let provider = rechecked_identity(&request.lock).map_err(|_| Error::LockChanged)?;
        if let Some(shell) = request.profile.shell_runtime()
            && rechecked_identity(shell.lock()).map_err(|_| Error::LockChanged)? != *shell.lock()
        {
            return Err(Error::LockChanged);
        }
        let helper_root = Path::new(&helper.canonical_path)
            .parent()
            .ok_or(Error::ProfileRejected)?;
        if !request.profile.resources().iter().any(|resource| {
            !resource.writable() && Path::new(&helper.canonical_path).starts_with(resource.root())
        }) {
            return Err(Error::ProfileRejected);
        }
        for root in std::iter::once(request.profile.workspace_root())
            .chain(
                request
                    .profile
                    .readable_roots()
                    .iter()
                    .map(|root| root.as_path()),
            )
            .chain(
                request
                    .profile
                    .writable_roots()
                    .iter()
                    .map(|root| root.as_path()),
            )
            .chain(
                request
                    .profile
                    .resources()
                    .iter()
                    .map(|resource| resource.root()),
            )
        {
            if std::fs::canonicalize(root).ok().as_deref() != Some(root) {
                return Err(Error::ProfileRejected);
            }
        }
        if helper != self.helper_lock
            || provider != request.lock
            || launch.lock != request.lock
            || launch.provider != provider.provider
            || launch.executable != Path::new(&provider.canonical_path)
            || launch.workspace_root.as_deref() != Some(request.profile.workspace_root())
        {
            return Err(Error::LockChanged);
        }
        let backend = SandboxBackendId::new("windows-psec-job-v1".into())
            .map_err(|_| Error::ContainmentRejected)?;
        request
            .attest_after_lock_recheck(&provider, backend.clone(), SandboxEnforcement::Enforced)
            .map_err(|_| Error::LockChanged)?;
        let mut command = config::command(request, launch, &helper, helper_root, run)?;
        let cleanup = cleanup::CleanupContext::new(request, helper.clone());
        let pending_pin = helper_image.try_clone().map_err(|_| Error::Unavailable)?;
        let child = command
            .spawn()
            .map_err(|error| Error::LaunchFailed(error.to_string()))?;
        let mut child = cleanup::PendingHelper::new(child, &cleanup, pending_pin);
        config::write_gateway_bootstrap(&mut child, request)?;
        let stdout = child.stdout.take().ok_or(Error::ContainmentRejected)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            let read_result = (&mut reader).take(1025).read_until(b'\n', &mut line);
            let result = read_result.map(|_| (reader, line));
            let _ = sender.send(result);
        });
        // Allow gateway admission and the contained temporary-directory bootstrap to finish.
        let handshake = receiver
            .recv_timeout(Duration::from_secs(45))
            .ok()
            .and_then(Result::ok)
            .and_then(|(reader, line)| {
                handshake(
                    &line,
                    &request.profile.digest_sha256(),
                    run,
                    request.profile.gateway().is_some(),
                )
                .map(|proof| (reader, proof))
            });
        let Some((stdout, (generation, peer))) = handshake else {
            return Err(child
                .failure()
                .map_or(Error::ContainmentRejected, Error::LaunchFailed));
        };
        let mut evidence =
            ProcessSandboxEvidence::new(request, &provider, run.into(), generation, backend)?;
        if let Some(peer) = peer {
            if request
                .profile
                .gateway()
                .is_none_or(|gateway| !peer.matches_active_gateway(gateway.peer_nonce()))
            {
                return Err(Error::ContainmentRejected);
            }
            evidence = evidence.with_network_peer(peer);
        }
        let stdin = child.stdin.take().ok_or(Error::ContainmentRejected)?;
        let stderr = child.stderr.take().ok_or(Error::ContainmentRejected)?;
        Ok(WindowsSandboxProcess {
            process: Some(SystemProcess::new(
                child.finish(),
                stdin,
                stdout,
                stderr,
                self.output_limit,
            )),
            forced: std::sync::atomic::AtomicBool::new(false),
            stop_lock: std::sync::Mutex::new(()),
            evidence,
            helper_image: Some(helper_image),
            cleanup,
        })
    }
}

fn validated_run<'a>(
    request: &SandboxedLaunchRequest,
    launch: &'a ProviderLaunch,
) -> Result<&'a str, Error> {
    request
        .profile
        .require_command_boundary(
            gent_types::AgentChatProvider::from_contained_id(&launch.provider).is_some(),
        )
        .map_err(|_| Error::ProfileRejected)?;
    match (request.profile.network(), request.profile.gateway()) {
        (SandboxNetworkPolicy::Disabled, None) => {}
        (
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256,
            },
            Some(gateway),
        ) if gateway.policy_digest_sha256() == policy_digest_sha256
            && gateway.api().provider_id() == launch.provider => {}
        _ => return Err(Error::ProfileRejected),
    }
    let run = launch
        .run_id
        .as_deref()
        .filter(|run| valid_run(run))
        .ok_or(Error::ProfileRejected)?;
    if request.profile.permission_binding() != launch.permission_binding.as_ref() {
        return Err(Error::ProfileRejected);
    }
    if request.profile.gateway().is_some() && request.profile.private_state().is_none() {
        return Err(Error::ProfileRejected);
    }
    if let Some(state) = request.profile.private_state() {
        if state.run_id() != run {
            return Err(Error::ProfileRejected);
        }
        for directory in state.directories() {
            if std::fs::canonicalize(&directory).ok().as_deref() != Some(directory.as_path()) {
                return Err(Error::ProfileRejected);
            }
        }
    }
    Ok(run)
}

#[path = "windows_sandbox_handshake.rs"]
mod handshake;
#[cfg(test)]
use handshake::generation;
use handshake::{handshake, valid_run};

impl WindowsSandboxProcess {
    fn owned(&self) -> &SystemProcess {
        self.process.as_ref().expect("owned sandbox process")
    }
    fn unprovable() -> ProcessTreeError {
        ProcessTreeError::Failed("native sandbox scope settlement is unprovable; retained ownership requires reconciliation".into())
    }
    fn settled(&self) -> Result<bool, ProcessTreeError> {
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        let status = self.owned().peek_owned_exit_status()?;
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        match status {
            Some(status) if !matches!(status.code(), Some(0 | 125)) => Err(Self::unprovable()),
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    fn stop_owned(&self, force: bool) -> Result<(), ProcessTreeError> {
        let _lock = self
            .stop_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        if self.settled()? {
            return Ok(());
        }
        if !force {
            self.owned().close_stdin()?;
            let deadline = std::time::Instant::now() + Duration::from_secs(18);
            while std::time::Instant::now() < deadline {
                if self.settled()? {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        // Reaping a killed helper is never evidence that either native Job is empty.
        self.forced
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = self.owned().kill_owned();
        let _ = self.owned().reap_owned();
        Err(Self::unprovable())
    }
}
impl ProcessTreeControl for WindowsSandboxProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.stop_owned(signal == ProcessTreeSignal::Kill)
    }
}
impl ProviderProcess for WindowsSandboxProcess {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        self.settled()
            .ok()
            .filter(|settled| !settled)
            .map(|_| &self.evidence)
    }
    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        if self.forced.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Self::unprovable());
        }
        self.owned().write_frame(frame)
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.owned().close_stdin()
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.owned().next_stdout_chunk()
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        if self.settled()? {
            self.owned().try_exit_code()
        } else {
            Ok(None)
        }
    }
}
impl Drop for WindowsSandboxProcess {
    fn drop(&mut self) {
        if self.stop_owned(false).is_ok() {
            let output = self.owned().output();
            for line in String::from_utf8_lossy(&output.stderr.bytes).lines() {
                if line.starts_with("GENT-SANDBOX-SETTLED ") {
                    eprintln!("{line}");
                }
            }
            self.cleanup.cleanup(self.evidence.network_peer());
        } else if let (Some(process), Some(pin)) = (self.process.take(), self.helper_image.take()) {
            // An unknown scope must not lose its process/image ownership just because a Rust wrapper drops.
            eprintln!(
                "native sandbox unprovable scope: {}",
                self.evidence.run_id()
            );
            cleanup::quarantine_process(process, pin);
        }
    }
}

#[cfg(test)]
#[path = "windows_sandbox_tests.rs"]
mod tests;

#[path = "windows_sandbox_cleanup.rs"]
mod cleanup;
#[path = "windows_sandbox_config.rs"]
mod config;
