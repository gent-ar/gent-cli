//! Native helper + cgroup process port. Production remains gated by exact contract.
use crate::{
    linux_cgroup_contract::ScopedHelperOwner,
    mxc_baseline::{self, CpuSemantics},
    translate,
};
use gent_drivers::{
    ProviderProcess,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    sandboxed_launch::{SandboxedProviderLaunch, SandboxedProviderLaunchError},
    supervisor::ProviderLaunch,
};
use gent_linux_cgroup_owner::{CgroupScope, DelegatedParent};
use gent_types::{RunVersionLock, SandboxedLaunchRequest};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub schema: u8,
    pub nonce: String,
    pub profile_digest: String,
    pub scope: String,
    pub parent_pid: u32,
    pub parent_birth: String,
    pub lock: RunVersionLock,
    pub helper_lock: RunVersionLock,
    pub executable: String,
    pub arguments: Vec<String>,
    pub cwd: String,
    pub readwrite: Vec<String>,
    pub readonly: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub wall_ms: u32,
    pub gateway_proxy: Option<std::net::SocketAddrV4>,
    pub runtime_root: Option<crate::network_runtime::RuntimePin>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Debug)]
pub struct Ready {
    pub schema: u8,
    pub nonce: String,
    pub profile_digest: String,
    pub scope: String,
    pub pid: u32,
    pub birth: String,
    pub worker_pid: u32,
    pub worker_birth: String,
    pub worker_namespaces: std::collections::BTreeMap<String, String>,
    pub sealed_image: crate::sealed_exec::SealedImageReady,
}
pub fn birth(pid: u32) -> Result<String, &'static str> {
    let text =
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|_| "process identity")?;
    Ok(text
        .rsplit_once(')')
        .ok_or("process identity")?
        .1
        .split_whitespace()
        .nth(19)
        .ok_or("process identity")?
        .to_owned())
}
fn error() -> ProcessTreeError {
    ProcessTreeError::Failed("Linux MXC owned transport failed".into())
}
pub struct GatewayAuthority {
    pub lease: Arc<dyn crate::gateway_bridge::LiveGatewayLease>,
    pub runtime: crate::network_runtime::RuntimePin,
}
#[derive(Debug)]
pub struct LinuxMxcLauncher {
    pub delegated_parent: PathBuf,
    pub helper_lock: RunVersionLock,
    pub gateway_authority: Option<GatewayAuthority>,
}
impl SandboxedProviderLaunch for LinuxMxcLauncher {
    type Process = LinuxMxcProcess;
    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
    ) -> Result<Self::Process, SandboxedProviderLaunchError> {
        if request.profile.permission_binding().is_none() {
            return Err(SandboxedProviderLaunchError::ProfileRejected);
        }
        translate(request, launch)?;
        mxc_baseline::validate(&request.profile)
            .map_err(|_| SandboxedProviderLaunchError::ProfileRejected)?;
        DelegatedParent::inspect(&self.delegated_parent)
            .map_err(|_| SandboxedProviderLaunchError::Unavailable)?;
        let gent_types::SandboxCpuSemantics::MonitoredTotal { poll_ms, wall_ms } =
            request.profile.cpu_semantics()
        else {
            return Err(SandboxedProviderLaunchError::LaunchFailed(
                "strict Gent CPU contract is not monitored CPU semantics".into(),
            ));
        };
        let (lease, runtime) = self
            .gateway_authority
            .as_ref()
            .map_or((None, None), |authority| {
                (
                    Some(authority.lease.clone()),
                    Some(authority.runtime.clone()),
                )
            });
        self.spawn_owned(
            request,
            launch,
            CpuSemantics::MonitoredTotal { poll_ms, wall_ms },
            lease,
            runtime,
        )
        .map_err(SandboxedProviderLaunchError::LaunchFailed)
    }
}
/// Historical execution observation for a contained READINESS PROBE only.
/// Issued solely after the backend observed the approved sealed image while
/// the exact worker was alive. Not live user-run evidence or an authorization.
#[derive(Debug)]
pub struct ProbeExecutionWitness<'a> {
    evidence: &'a gent_drivers::sandboxed_launch::ProcessSandboxEvidence,
}
impl ProbeExecutionWitness<'_> {
    pub fn matches(&self, request: &SandboxedLaunchRequest, run_id: &str) -> bool {
        self.evidence.matches(request, run_id)
    }
}
pub struct LinuxMxcProcess {
    owner: Arc<Mutex<ScopedHelperOwner>>,
    input: Mutex<Option<File>>,
    output: Mutex<File>,
    stderr: Mutex<File>,
    pending: Mutex<VecDeque<Vec<u8>>>,
    diagnostic: Mutex<Vec<u8>>,
    stop: Option<mpsc::Sender<()>>,
    watch: Option<thread::JoinHandle<()>>,
    gateway: Option<crate::gateway_bridge::GatewayBridge>,
    gate_proof: Arc<crate::contained_gate::KernelGateProof>,
    evidence: Option<gent_drivers::sandboxed_launch::ProcessSandboxEvidence>,
}
impl LinuxMxcProcess {
    pub fn probe_execution_witness(&self) -> Option<ProbeExecutionWitness<'_>> {
        if !self.gate_proof.executed() {
            return None;
        }
        self.evidence
            .as_ref()
            .map(|evidence| ProbeExecutionWitness { evidence })
    }
}
impl LinuxMxcLauncher {
    fn spawn_owned(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
        cpu: CpuSemantics,
        daemon: Option<Arc<dyn crate::gateway_bridge::LiveGatewayLease>>,
        runtime_root: Option<crate::network_runtime::RuntimePin>,
    ) -> Result<LinuxMxcProcess, String> {
        if request.profile.gateway().is_some() != (daemon.is_some() && runtime_root.is_some()) {
            return Err("gateway requires retained daemon lease and pinned runtime".into());
        }
        if let Some(runtime) = &runtime_root {
            runtime.verify().map_err(str::to_owned)?;
        }
        let CpuSemantics::MonitoredTotal { poll_ms, wall_ms } =
            cpu.validate().map_err(str::to_owned)?
        else {
            return Err("CPU policy".into());
        };
        mxc_baseline::validate(&request.profile)
            .map_err(|e| format!("explicit OS baseline not covered: {e:?}"))?;
        let profile = translate(request, launch).map_err(|_| "launch/profile mismatch")?;
        crate::immutable_image::require(std::path::Path::new(&self.helper_lock.canonical_path))
            .map_err(str::to_owned)?;
        if gent_drivers::lock::rechecked_identity(&self.helper_lock).map_err(|_| "helper fence")?
            != self.helper_lock
        {
            return Err("helper changed".into());
        }
        let parent = DelegatedParent::inspect(&self.delegated_parent)
            .map_err(|_| "delegation unavailable")?;
        let nonce = format!(
            "{:032x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "clock")?
                .as_nanos()
        );
        let begun = Instant::now();
        let scope = CgroupScope::create(parent, &nonce, request.profile.limits())
            .map_err(|_| "scope setup")?;
        let mut start = Start {
            schema: 1,
            nonce: nonce.clone(),
            profile_digest: request.profile.digest_sha256(),
            scope: scope.membership_path().map_err(|_| "scope identity")?,
            parent_pid: std::process::id(),
            parent_birth: birth(std::process::id()).map_err(str::to_owned)?,
            lock: request.lock.clone(),
            helper_lock: self.helper_lock.clone(),
            executable: profile.executable,
            arguments: profile.arguments,
            cwd: profile.cwd,
            readwrite: profile.readwrite,
            readonly: profile.readonly,
            environment: profile.environment,
            wall_ms,
            gateway_proxy: None,
            runtime_root,
        };
        let (child, pid, generation) = spawn_helper(&self.helper_lock, &start)?;
        let owner = ScopedHelperOwner::adopt(
            scope,
            child,
            request,
            launch,
            &self.helper_lock,
            generation.clone(),
        )
        .map_err(str::to_owned)?;
        let owner = Arc::new(Mutex::new(owner));
        let live_daemon = daemon.clone();
        let gateway = start_gateway(request, daemon, &owner)?;
        start.gateway_proxy = gateway
            .as_ref()
            .map(crate::gateway_bridge::GatewayBridge::endpoint);
        let (input, mut output, stderr) = release_startup(&owner, &start, begun, wall_ms)?;
        let (gate_proof, initial) = await_ready(
            &owner,
            &mut output,
            &start,
            &ReadyExpectation {
                pid,
                generation: &generation,
                begun,
                wall_ms,
                live_daemon: live_daemon.as_ref(),
            },
        )?;
        let gate_proof = Arc::new(gate_proof);
        let evidence = Some(startup_evidence(
            request,
            launch,
            &owner,
            &gate_proof,
            gateway.as_ref(),
            gent_types::SandboxCpuSemantics::MonitoredTotal { poll_ms, wall_ms },
        )?);
        if begun.elapsed() >= Duration::from_millis(u64::from(wall_ms))
            || owner
                .lock()
                .map_err(|_| "owner mutex")?
                .poll()
                .map_err(str::to_owned)?
                .is_some()
        {
            return Err("startup exhausted resource budget before provider release".into());
        }
        let (stop, rx) = mpsc::channel();
        let watch = OwnerWatch {
            owner: Arc::downgrade(&owner),
            worker: gate_proof.clone(),
            gateway_live: gateway
                .as_ref()
                .map(crate::gateway_bridge::GatewayBridge::liveness),
            gateway_cancel: gateway
                .as_ref()
                .map(crate::gateway_bridge::GatewayBridge::cancellation),
            daemon: live_daemon.clone(),
            begun,
            poll_ms,
            wall_ms,
        }
        .spawn(rx);
        let mut input = input;
        let execution_deadline = (Instant::now() + Duration::from_secs(3))
            .min(begun + Duration::from_millis(u64::from(wall_ms)));
        if let Err(failure) =
            release_worker(&mut input, &start.nonce, &gate_proof, execution_deadline)
        {
            let _ = stop.send(());
            let _ = owner
                .lock()
                .map_err(|_| "owner mutex")?
                .finish(Duration::from_secs(2));
            let _ = watch.join();
            return Err(failure.into());
        }
        Ok(LinuxMxcProcess {
            owner,
            input: Mutex::new(Some(input)),
            output: Mutex::new(output),
            stderr: Mutex::new(stderr),
            pending: Mutex::new(initial),
            diagnostic: Mutex::new(Vec::new()),
            stop: Some(stop),
            watch: Some(watch),
            gateway,
            gate_proof,
            evidence,
        })
    }
}
fn start_gateway(
    request: &SandboxedLaunchRequest,
    daemon: Option<Arc<dyn crate::gateway_bridge::LiveGatewayLease>>,
    owner: &Arc<Mutex<ScopedHelperOwner>>,
) -> Result<Option<crate::gateway_bridge::GatewayBridge>, String> {
    let (Some(descriptor), Some(daemon)) = (request.profile.gateway(), daemon) else {
        return Ok(None);
    };
    let lease = Arc::new(crate::gateway_bridge::OwnerBoundLease {
        owner: Arc::downgrade(owner),
        daemon,
    });
    crate::gateway_bridge::GatewayBridge::start(descriptor.endpoint(), lease)
        .map(Some)
        .map_err(str::to_owned)
}

fn spawn_helper(
    helper: &RunVersionLock,
    start: &Start,
) -> Result<(std::process::Child, u32, String), String> {
    let mut command = Command::new(&helper.canonical_path);
    command.env_clear().env("PATH", "/usr/bin:/bin");
    if let Some(runtime) = &start.runtime_root {
        command.envs(runtime.helper_environment().map_err(str::to_owned)?);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "helper spawn")?;
    let pid = child.id();
    let Ok(generation) = birth(pid) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("helper identity".into());
    };
    Ok((child, pid, generation))
}

fn release_startup(
    owner: &Mutex<ScopedHelperOwner>,
    start: &Start,
    begun: Instant,
    wall_ms: u32,
) -> Result<(File, File, File), String> {
    let mut bytes = serde_json::to_vec(start).map_err(|_| "startup encode")?;
    bytes.push(b'\n');
    if bytes.len() > 65536 {
        return Err("startup bound".into());
    }
    let mut locked = owner.lock().map_err(|_| "owner mutex")?;
    locked
        .release_streaming_before(
            &bytes,
            (Instant::now() + Duration::from_millis(500))
                .min(begun + Duration::from_millis(u64::from(wall_ms))),
        )
        .map_err(str::to_owned)?;
    locked.take_streaming_pipes().map_err(str::to_owned)
}

fn release_worker(
    input: &mut File,
    nonce: &str,
    gate_proof: &crate::contained_gate::KernelGateProof,
    deadline: Instant,
) -> Result<(), &'static str> {
    input
        .write_all(format!("GENT-PROVIDER-GO/1 {nonce}\n").as_bytes())
        .map_err(|_| "worker release")?;
    while Instant::now() < deadline {
        if gate_proof.current_status().is_err() {
            break;
        }
        if gate_proof.executed() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Err("sealed provider execution not observed")
}

struct ReadyExpectation<'a> {
    pid: u32,
    generation: &'a str,
    begun: Instant,
    wall_ms: u32,
    live_daemon: Option<&'a Arc<dyn crate::gateway_bridge::LiveGatewayLease>>,
}

fn await_ready(
    owner: &Mutex<ScopedHelperOwner>,
    output: &mut File,
    start: &Start,
    expect: &ReadyExpectation<'_>,
) -> Result<(crate::contained_gate::KernelGateProof, VecDeque<Vec<u8>>), String> {
    let mut ready = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut initial = VecDeque::new();
    loop {
        if expect.begun.elapsed() >= Duration::from_millis(u64::from(expect.wall_ms))
            || expect.live_daemon.is_some_and(|v| !v.current())
        {
            return Err("startup wall or lease expired".into());
        }
        if owner
            .lock()
            .map_err(|_| "owner mutex")?
            .poll()
            .map_err(str::to_owned)?
            .is_some()
        {
            return Err("helper exited during startup CPU enforcement".into());
        }
        let mut buffer = [0; 4096];
        match output.read(&mut buffer) {
            Ok(0) => return Err("helper startup EOF".into()),
            Ok(n) => {
                ready.extend_from_slice(&buffer[..n]);
                if ready.len() > 65536 {
                    return Err("ready bound".into());
                }
                if let Some(end) = ready.iter().position(|b| *b == b'\n') {
                    if end + 1 < ready.len() {
                        initial.push_back(ready[end + 1..].to_vec());
                    }
                    let ack: Ready =
                        serde_json::from_slice(&ready[..end]).map_err(|_| "ready invalid")?;
                    if ack.schema != 2
                        || ack.nonce != start.nonce
                        || ack.profile_digest != start.profile_digest
                        || ack.scope != start.scope
                        || ack.pid != expect.pid
                        || ack.birth != expect.generation
                    {
                        return Err("ready identity mismatch".into());
                    }
                    let proof = crate::contained_gate::inspect(
                        start,
                        &crate::contained_gate::GateReady {
                            schema: 2,
                            nonce: ack.nonce.clone(),
                            namespaces: ack.worker_namespaces,
                            sealed_image: ack.sealed_image,
                        },
                    )
                    .map_err(str::to_owned)?;
                    if proof.pid != ack.worker_pid || proof.birth != ack.worker_birth {
                        return Err("worker proof mismatch".into());
                    }
                    return Ok((proof, initial));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2))
            }
            _ => return Err("ready timeout".into()),
        }
    }
}

fn startup_evidence(
    request: &SandboxedLaunchRequest,
    launch: &ProviderLaunch,
    owner: &Mutex<ScopedHelperOwner>,
    gate_proof: &crate::contained_gate::KernelGateProof,
    gateway: Option<&crate::gateway_bridge::GatewayBridge>,
    cpu: gent_types::SandboxCpuSemantics,
) -> Result<gent_drivers::sandboxed_launch::ProcessSandboxEvidence, String> {
    if request.profile.cpu_semantics() != cpu {
        return Err("CPU semantics mismatch".into());
    }
    {
        let mut locked = owner.lock().map_err(|_| "owner mutex")?;
        if !locked.resource_limits_match(request.profile.limits())
            || !locked.gateway_owner_current()
        {
            return Err("worker owner unavailable".into());
        }
    }
    let current =
        gent_drivers::lock::rechecked_identity(&request.lock).map_err(|_| "provider changed")?;
    let mut evidence = gent_drivers::sandboxed_launch::ProcessSandboxEvidence::new(
        request,
        &current,
        launch.run_id.clone().ok_or("run binding")?,
        gate_proof.birth.clone(),
        gent_types::SandboxBackendId::new("linux-mxc-cgroup-monitored-v1".into())
            .map_err(|_| "backend identity")?,
    )
    .map_err(|_| "attestation rejected")?;
    if let (Some(descriptor), Some(bridge)) = (request.profile.gateway(), gateway) {
        let peer = gent_drivers::SandboxNetworkPeer::linux_owned_proxy(
            request.profile.digest_sha256(),
            descriptor.peer_nonce().into(),
            bridge.endpoint().port(),
            std::process::id(),
            birth(std::process::id())
                .map_err(str::to_owned)?
                .parse()
                .map_err(|_| "peer birth")?,
        )
        .ok_or("network peer")?;
        evidence = evidence.with_network_peer(peer);
    }
    Ok(evidence)
}

struct OwnerWatch {
    owner: std::sync::Weak<Mutex<ScopedHelperOwner>>,
    worker: Arc<crate::contained_gate::KernelGateProof>,
    gateway_live: Option<Arc<std::sync::atomic::AtomicBool>>,
    gateway_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    daemon: Option<Arc<dyn crate::gateway_bridge::LiveGatewayLease>>,
    begun: Instant,
    poll_ms: u16,
    wall_ms: u32,
}

impl OwnerWatch {
    fn spawn(self, rx: mpsc::Receiver<()>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut exited_since = None;
            while rx
                .recv_timeout(Duration::from_millis(u64::from(self.poll_ms)))
                .is_err()
            {
                let Some(owner) = self.owner.upgrade() else {
                    break;
                };
                let worker_invalid = self.worker.current_status().is_err();
                let worker_exited = self.worker.exited();
                if worker_invalid || worker_exited {
                    if let Some(cancel) = &self.gateway_cancel {
                        cancel.store(true, std::sync::atomic::Ordering::Release);
                    }
                }
                let grace_elapsed = if worker_exited {
                    exited_since.get_or_insert_with(Instant::now).elapsed()
                        >= Duration::from_millis(100)
                } else {
                    false
                };
                let worker_lost = (worker_invalid && !worker_exited) || grace_elapsed;
                let lease_lost = self.daemon.as_ref().is_some_and(|lease| !lease.current())
                    || (!worker_exited
                        && self.gateway_live.as_ref().is_some_and(|alive| {
                            !alive.load(std::sync::atomic::Ordering::Acquire)
                        }));
                let mut owner = owner.lock().unwrap_or_else(|e| e.into_inner());
                if lease_lost || worker_lost {
                    let _ = owner.finish(Duration::from_secs(2));
                    break;
                }
                if self.begun.elapsed() >= Duration::from_millis(u64::from(self.wall_ms)) {
                    let _ = owner.finish(Duration::from_secs(2));
                    break;
                }
                match owner.poll() {
                    Ok(Some(_)) | Err(_) => {
                        let _ = owner.finish(Duration::from_secs(2));
                        break;
                    }
                    _ => {}
                }
            }
        })
    }
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug)]
pub struct PublicKernelSnapshot {
    pub backend: String,
    pub profile_digest: String,
    pub provider_digest: String,
    pub helper_pid: u32,
    pub helper_birth: String,
    pub worker_pid: u32,
    pub worker_birth: String,
    pub namespaces: std::collections::BTreeMap<String, String>,
    pub user_id: u32,
    pub cap_eff: u64,
    pub no_new_privs: bool,
    pub scope: String,
    pub memory_max: u64,
    pub pids_max: u16,
    pub cpu_budget_usec: u64,
    pub local_gateway_port: Option<u16>,
    pub sealed_image: serde_json::Value,
}
impl LinuxMxcProcess {
    pub fn public_kernel_snapshot(&self) -> Result<PublicKernelSnapshot, &'static str> {
        if self
            .gateway
            .as_ref()
            .is_some_and(|gateway| !gateway.current())
        {
            return Err("gateway inactive");
        }
        let mut owner = self.owner.lock().map_err(|_| "owner unavailable")?;
        if !owner.gateway_owner_current() {
            return Err("owner inactive");
        }
        let (helper_pid, scope, memory_max, pids_max, cpu_budget_usec) =
            owner.public_kernel_state()?;
        if birth(self.gate_proof.pid).ok().as_deref() != Some(self.gate_proof.birth.as_str()) {
            return Err("worker inactive");
        }
        let proof = self.evidence.as_ref().ok_or("proof unavailable")?;
        let status = self.gate_proof.current_status()?;
        let cap_eff = u64::from_str_radix(
            status
                .lines()
                .find_map(|line| line.strip_prefix("CapEff:\t"))
                .ok_or("worker capabilities")?,
            16,
        )
        .map_err(|_| "worker capabilities")?;
        let no_new_privs = status.lines().any(|line| line == "NoNewPrivs:\t1");
        Ok(PublicKernelSnapshot {
            backend: proof.attestation().backend.as_str().into(),
            profile_digest: owner.profile_digest().into(),
            provider_digest: proof.attestation().executable_digest_sha256.clone(),
            helper_pid,
            helper_birth: owner.helper_generation().into(),
            worker_pid: self.gate_proof.pid,
            worker_birth: self.gate_proof.birth.clone(),
            namespaces: crate::contained_gate::namespaces(self.gate_proof.pid)?,
            user_id: status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:\t"))
                .and_then(|line| line.split_whitespace().next())
                .and_then(|v| v.parse().ok())
                .ok_or("worker UID unavailable")?,
            cap_eff,
            no_new_privs,
            scope,
            memory_max,
            pids_max,
            cpu_budget_usec,
            sealed_image: self.gate_proof.image_proof()?,
            local_gateway_port: self
                .gateway
                .as_ref()
                .map(|gateway| gateway.endpoint().port()),
        })
    }

    pub fn diagnostic_owner_current(&self) -> bool {
        if self.gate_proof.current_status().is_err() {
            return false;
        }
        if self
            .gateway
            .as_ref()
            .is_some_and(|gateway| !gateway.current())
        {
            return false;
        }
        self.owner
            .lock()
            .is_ok_and(|mut owner| owner.gateway_owner_current())
    }
    fn settle(&self) -> Result<(), ProcessTreeError> {
        self.owner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finish(Duration::from_secs(2))
            .map_err(|_| error())
    }
    fn drain_error(&self) {
        let mut file = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        let mut bytes = [0; 4096];
        for _ in 0..16 {
            match file.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    let mut kept = self.diagnostic.lock().unwrap_or_else(|e| e.into_inner());
                    let count = (65536 - kept.len()).min(n);
                    kept.extend_from_slice(&bytes[..count]);
                }
                _ => break,
            }
        }
    }
    pub fn gateway_diagnostic(&self) -> std::collections::BTreeMap<String, u64> {
        self.gateway
            .as_ref()
            .map(|g| g.diagnostic())
            .unwrap_or_default()
    }
    pub fn diagnostic_bytes(&self) -> Vec<u8> {
        self.diagnostic
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
impl ProcessTreeControl for LinuxMxcProcess {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        if signal == ProcessTreeSignal::Interrupt {
            return Err(ProcessTreeError::Failed(
                "soft interrupt not implemented".into(),
            ));
        }
        self.settle()
    }
}
impl ProviderProcess for LinuxMxcProcess {
    fn sandbox_evidence(&self) -> Option<&gent_drivers::sandboxed_launch::ProcessSandboxEvidence> {
        if self.diagnostic_owner_current() {
            self.evidence.as_ref()
        } else {
            None
        }
    }

    fn write_frame(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        if frame.len() > 1024 * 1024 {
            return Err(error());
        }
        let mut input = self.input.lock().unwrap_or_else(|e| e.into_inner());
        let file = input.as_mut().ok_or_else(error)?;
        let start = Instant::now();
        let mut at = 0;
        while at < frame.len() {
            match file.write(&frame[at..]) {
                Ok(n) if n > 0 => at += n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed() < Duration::from_millis(500) =>
                {
                    thread::sleep(Duration::from_millis(2))
                }
                _ => {
                    let _ = self.settle();
                    return Err(error());
                }
            }
        }
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.input.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok(())
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        self.drain_error();
        if let Some(bytes) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
        {
            return Ok(Some(bytes));
        }
        let mut bytes = vec![0; 4096];
        match self
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read(&mut bytes)
        {
            Ok(0) => Ok(None),
            Ok(n) => {
                bytes.truncate(n);
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            _ => Err(error()),
        }
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        let status = self
            .owner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .poll()
            .map_err(|_| error())?;
        if let Some(status) = status {
            self.settle()?;
            self.drain_error();
            let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
            let mut bytes = [0; 4096];
            let mut total = 0;
            loop {
                match output.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if total > 2 * 1024 * 1024 {
                            return Err(error());
                        }
                        self.pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push_back(bytes[..n].to_vec());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    _ => return Err(error()),
                }
            }
            return Ok(Some(status.code()));
        }
        Ok(None)
    }
}
impl Drop for LinuxMxcProcess {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.settle();
        self.gateway.take();
        if let Some(watch) = self.watch.take() {
            let _ = watch.join();
        }
    }
}

impl std::fmt::Debug for Start {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Start")
            .field("schema", &self.schema)
            .field("environment", &"[REDACTED]")
            .field("arguments", &self.arguments.len())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for GatewayAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayAuthority")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for LinuxMxcProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxMxcProcess").finish_non_exhaustive()
    }
}
