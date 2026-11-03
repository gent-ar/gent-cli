//! Retained helper image/process/cgroup owner. No detached user payload.
use crate::translate;
use gent_drivers::{sandboxed_launch::SandboxedProviderLaunchError, supervisor::ProviderLaunch};
use gent_linux_cgroup_owner::CgroupScope;
use gent_types::{RunVersionLock, SandboxedLaunchRequest};
use std::path::PathBuf;
use std::{
    fs::File,
    io::Write,
    os::unix::fs::MetadataExt,
    process::{Child, ExitStatus, Output},
    time::{Duration, Instant},
};
/// Owns the blocked helper and its kernel scope together. Diagnostic only until
/// the complete Gent attestation contract is supported; no detached readers.
#[derive(Debug)]
pub struct ScopedHelperOwner {
    scope: Option<CgroupScope>,
    helper: Option<Child>,
    _image: File,
    profile_digest: String,
    generation: String,
    released: bool,
    settled: bool,
    limits: gent_types::SandboxResourceLimits,
}
impl ScopedHelperOwner {
    /// Live ownership check, deliberately not an Enforced attestation. Every
    /// gateway request must retain the real daemon RunFence as a second condition.
    pub fn resource_limits_match(&self, limits: gent_types::SandboxResourceLimits) -> bool {
        self.scope.as_ref().is_some_and(|scope| {
            scope.memory_limit_bytes().ok() == Some(limits.max_memory_bytes)
                && scope.task_limit().ok() == Some(limits.max_processes)
                && limits.max_cpu_time_ms.checked_mul(1000)
                    == Some(scope.monitored_cpu_budget_usec())
        })
    }
    pub fn gateway_owner_current(&mut self) -> bool {
        if self.settled {
            return false;
        }
        let Some(helper) = self.helper.as_mut() else {
            return false;
        };
        if !matches!(helper.try_wait(), Ok(None)) {
            return false;
        }
        let pid = helper.id();
        if crate::native_runtime::birth(pid).ok().as_deref() != Some(self.generation.as_str()) {
            return false;
        }
        let Some(scope) = self.scope.as_ref() else {
            return false;
        };
        let Ok(path) = scope.membership_path() else {
            return false;
        };
        if std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
            .ok()
            .as_deref()
            .map(str::trim)
            != Some(format!("0::{path}").as_str())
        {
            return false;
        }
        scope.memory_limit_bytes().ok() == Some(self.limits.max_memory_bytes)
            && scope.task_limit().ok() == Some(self.limits.max_processes)
            && self.limits.max_cpu_time_ms.checked_mul(1000)
                == Some(scope.monitored_cpu_budget_usec())
    }
    pub fn release_streaming_once(&mut self, frame: &[u8]) -> Result<(), &'static str> {
        self.release_streaming_before(frame, Instant::now() + Duration::from_millis(500))
    }
    pub fn release_streaming_before(
        &mut self,
        frame: &[u8],
        deadline: Instant,
    ) -> Result<(), &'static str> {
        if self.released || self.settled || frame.is_empty() || frame.len() > 65536 {
            return Err("invalid helper startup frame");
        }
        let scope = self.scope.as_ref().ok_or("scope unavailable")?;
        let input = self
            .helper
            .as_mut()
            .ok_or("helper unavailable")?
            .stdin
            .as_mut()
            .ok_or("helper input unavailable")?;
        let flags = rustix::fs::fcntl_getfl(&*input).map_err(|_| "pipe flags")?;
        rustix::fs::fcntl_setfl(&*input, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "pipe flags")?;
        let mut offset = 0;
        while offset < frame.len() {
            if Instant::now() >= deadline
                || scope
                    .sample_cpu_budget()
                    .map_err(|_| "startup CPU accounting")?
                    .exceeded
            {
                return Err("startup wall or CPU budget exceeded");
            }
            match input.write(&frame[offset..]) {
                Ok(n) if n > 0 => offset += n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2))
                }
                _ => return Err("helper startup pipe stalled"),
            }
        }
        self.released = true;
        Ok(())
    }
    pub fn take_streaming_pipes(&mut self) -> Result<(File, File, File), &'static str> {
        if !self.released || self.settled {
            return Err("helper startup not released");
        }
        let child = self.helper.as_mut().ok_or("helper unavailable")?;
        let input = File::from(std::os::fd::OwnedFd::from(
            child.stdin.take().ok_or("stdin unavailable")?,
        ));
        let output = File::from(std::os::fd::OwnedFd::from(
            child.stdout.take().ok_or("stdout unavailable")?,
        ));
        let error = File::from(std::os::fd::OwnedFd::from(
            child.stderr.take().ok_or("stderr unavailable")?,
        ));
        for fd in [&input, &output, &error] {
            let flags = rustix::fs::fcntl_getfl(fd).map_err(|_| "pipe flags")?;
            rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)
                .map_err(|_| "pipe flags")?;
        }
        Ok((input, output, error))
    }
    pub fn adopt(
        scope: CgroupScope,
        mut helper: Child,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
        expected_image: &RunVersionLock,
        generation: String,
    ) -> Result<Self, &'static str> {
        let checked = (|| {
            scope_matches(&scope, request, launch).map_err(|_| "request/scope mismatch")?;
            let proc_image = PathBuf::from(format!("/proc/{}/exe", helper.id()));
            let image = File::open(&proc_image).map_err(|_| "helper image unavailable")?;
            let metadata = image.metadata().map_err(|_| "helper image unavailable")?;
            let parent = std::fs::metadata(
                PathBuf::from(&expected_image.canonical_path)
                    .parent()
                    .ok_or("helper image parent unavailable")?,
            )
            .map_err(|_| "helper image parent unavailable")?;
            if metadata.uid() != 0
                || metadata.mode() & 0o022 != 0
                || parent.uid() != 0
                || parent.mode() & 0o022 != 0
            {
                return Err("helper image is not immutable to this caller");
            }
            let observed = gent_drivers::lock::capture(
                &expected_image.provider,
                &proc_image,
                &expected_image.version,
                &expected_image.compatibility_entry,
            )
            .map_err(|_| "helper image fence unavailable")?;
            if &observed != expected_image {
                return Err("helper image changed");
            }
            scope
                .attach_blocked_helper(&mut helper, &generation)
                .map_err(|_| "helper generation/membership rejected")?;
            Ok(image)
        })();
        match checked {
            Ok(image) => Ok(Self {
                scope: Some(scope),
                helper: Some(helper),
                _image: image,
                profile_digest: request.profile.digest_sha256(),
                generation,
                released: false,
                settled: false,
                limits: request.profile.limits(),
            }),
            Err(error) => {
                let _ = helper.kill();
                let until = Instant::now() + Duration::from_secs(1);
                while Instant::now() < until {
                    if helper.try_wait().ok().flatten().is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error)
            }
        }
    }
    pub fn public_kernel_state(&self) -> Result<(u32, String, u64, u16, u64), &'static str> {
        let scope = self.scope.as_ref().ok_or("scope unavailable")?;
        Ok((
            self.helper.as_ref().ok_or("helper unavailable")?.id(),
            scope.membership_path().map_err(|_| "scope unavailable")?,
            scope
                .memory_limit_bytes()
                .map_err(|_| "memory unavailable")?,
            scope.task_limit().map_err(|_| "tasks unavailable")?,
            scope.monitored_cpu_budget_usec(),
        ))
    }
    pub fn profile_digest(&self) -> &str {
        &self.profile_digest
    }
    pub fn helper_generation(&self) -> &str {
        &self.generation
    }
    pub fn release_once(&mut self, frame: &[u8]) -> Result<(), &'static str> {
        if self.released || self.settled || frame.is_empty() || frame.len() > 4096 {
            return Err("invalid or repeated helper release");
        }
        let helper = self.helper.as_mut().ok_or("helper unavailable")?;
        if helper
            .try_wait()
            .map_err(|_| "helper status unavailable")?
            .is_some()
        {
            return Err("helper already exited");
        }
        let mut input = helper.stdin.take().ok_or("helper input unavailable")?;
        let flags = rustix::fs::fcntl_getfl(&input).map_err(|_| "helper pipe flags unavailable")?;
        rustix::fs::fcntl_setfl(&input, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "helper pipe flags unavailable")?;
        if input.write(frame).ok() != Some(frame.len()) {
            let _ = self.finish(Duration::from_secs(2));
            return Err("helper startup frame failed");
        }
        self.released = true;
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<ExitStatus>, &'static str> {
        if !self.settled && !self.resource_limits_match(self.limits) {
            self.finish(Duration::from_secs(2))?;
            return Err("live resource controls changed");
        }
        if !self.settled
            && self
                .scope
                .as_ref()
                .ok_or("scope unavailable")?
                .sample_cpu_budget()
                .map_err(|_| "CPU accounting unavailable")?
                .exceeded
        {
            self.finish(Duration::from_secs(2))?;
        }
        self.helper
            .as_mut()
            .ok_or("helper unavailable")?
            .try_wait()
            .map_err(|_| "helper status unavailable")
    }
    pub fn peak_tasks(&self) -> Result<u64, &'static str> {
        self.scope
            .as_ref()
            .ok_or("scope unavailable")?
            .peak_tasks()
            .map_err(|_| "scope task accounting unavailable")
    }
    pub fn finish(&mut self, deadline: Duration) -> Result<(), &'static str> {
        if self.settled {
            return Ok(());
        }
        let cleanup = self
            .scope
            .as_mut()
            .ok_or("scope unavailable")?
            .terminate_and_remove(deadline)
            .map_err(|_| "scope settlement incomplete")?;
        if !cleanup.empty || !cleanup.removed {
            return Err("scope settlement incomplete");
        }
        let until = Instant::now() + deadline;
        let helper = self.helper.as_mut().ok_or("helper unavailable")?;
        loop {
            if helper
                .try_wait()
                .map_err(|_| "helper status unavailable")?
                .is_some()
            {
                self.settled = true;
                return Ok(());
            }
            if Instant::now() >= until {
                return Err("helper reap incomplete");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub fn take_output_after_settlement(&mut self) -> Result<Output, &'static str> {
        if !self.settled {
            return Err("scope must settle before joining pipes");
        }
        self.helper
            .take()
            .ok_or("helper output already consumed")?
            .wait_with_output()
            .map_err(|_| "helper output unavailable")
    }
}
impl Drop for ScopedHelperOwner {
    fn drop(&mut self) {
        if !self.settled && self.helper.is_some() {
            let _ = self.finish(Duration::from_millis(500));
        }
    }
}
fn scope_matches(
    scope: &CgroupScope,
    request: &SandboxedLaunchRequest,
    launch: &ProviderLaunch,
) -> Result<(), SandboxedProviderLaunchError> {
    let _profile = translate(request, launch)?;
    let limits = request.profile.limits();
    if scope.memory_limit_bytes().ok() != Some(limits.max_memory_bytes)
        || scope.task_limit().ok() != Some(limits.max_processes)
        || limits.max_cpu_time_ms.checked_mul(1000) != Some(scope.monitored_cpu_budget_usec())
    {
        return Err(SandboxedProviderLaunchError::ContainmentRejected);
    }
    Ok(())
}
