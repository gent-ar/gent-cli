use super::*;
use gent_drivers::{
    ProviderProcess,
    interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal},
    sandboxed_launch::ProcessSandboxEvidence,
};
use gent_types::*;
use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
pub(super) struct Fake {
    proof: ProcessSandboxEvidence,
    output: Mutex<VecDeque<Vec<u8>>>,
    pub(super) writes: AtomicUsize,
    exit: Mutex<Option<i32>>,
    unknown: AtomicBool,
    known: bool,
    blocked: bool,
    gate: (Mutex<(bool, bool)>, Condvar),
    kills: AtomicUsize,
}
impl ProcessTreeControl for Fake {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        if signal == ProcessTreeSignal::Kill {
            self.kills.fetch_add(1, Ordering::AcqRel);
        }
        let mut g = self.gate.0.lock().unwrap();
        g.1 = true;
        self.gate.1.notify_all();
        if self.known && signal != ProcessTreeSignal::Kill {
            *self.exit.lock().unwrap() = Some(0);
            Ok(())
        } else {
            self.unknown.store(true, Ordering::Release);
            Err(ProcessTreeError::Failed("unknown fixture scope".into()))
        }
    }
}
impl ProviderProcess for Fake {
    fn sandbox_evidence(&self) -> Option<&ProcessSandboxEvidence> {
        if self.unknown.load(Ordering::Acquire) || self.exit.lock().unwrap().is_some() {
            None
        } else {
            Some(&self.proof)
        }
    }
    fn write_frame(&self, b: &[u8]) -> Result<(), ProcessTreeError> {
        let count = self.writes.fetch_add(1, Ordering::AcqRel);
        if count > 0 && self.blocked {
            let mut g = self.gate.0.lock().unwrap();
            g.0 = true;
            self.gate.1.notify_all();
            while !g.1 {
                g = self.gate.1.wait(g).unwrap()
            }
            return Err(ProcessTreeError::Failed("cancelled fixture write".into()));
        }
        if !b.starts_with(b"GENTSR01") {
            let meta=serde_json::to_vec(&serde_json::json!({"status":200,"contentType":"application/octet-stream","headers":{},"stream":false})).unwrap();
            let mut bytes = u32::try_from(meta.len()).unwrap().to_le_bytes().to_vec();
            bytes.extend(meta);
            bytes.extend(9u32.to_le_bytes());
            bytes.extend(b"GENT-UNIT");
            bytes.extend(0u32.to_le_bytes());
            self.output.lock().unwrap().push_back(bytes);
        }
        Ok(())
    }
    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        self.signal_tree(ProcessTreeSignal::Terminate)
    }
    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.output.lock().unwrap().pop_front())
    }
    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        if self.unknown.load(Ordering::Acquire) {
            Err(ProcessTreeError::Failed("unknown fixture".into()))
        } else {
            Ok(self.exit.lock().unwrap().map(Some))
        }
    }
}
pub(super) fn fixture(
    known: bool,
    blocked: bool,
) -> (
    std::path::PathBuf,
    Arc<Fake>,
    SandboxedLaunchRequest,
    ModelResources,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let image = path.join("worker.exe");
    std::fs::write(&image, b"nonexecuted fake image").unwrap();
    let lock = gent_drivers::lock::capture("claurst-model", &image, "1", "fake").unwrap();
    let profile = SandboxLaunchProfile::new(
        &path,
        std::slice::from_ref(&path),
        &[],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 2,
            max_memory_bytes: 1024 * 1024,
            max_cpu_time_ms: 1000,
        },
    )
    .unwrap();
    let request = SandboxedLaunchRequest {
        lock: lock.clone(),
        profile,
    };
    let proof = ProcessSandboxEvidence::new(
        &request,
        &lock,
        "run".into(),
        "fake-generation".into(),
        SandboxBackendId::new("unit-only".into()).unwrap(),
    )
    .unwrap();
    let mut ready = b"GENTSD01".to_vec();
    ready.extend(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    #[cfg(windows)]
    let pin = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&image)
            .unwrap()
    };
    #[cfg(not(windows))]
    let pin = std::fs::File::open(&image).unwrap();
    let fake = Arc::new(Fake {
        proof,
        output: Mutex::new(VecDeque::from([ready])),
        writes: AtomicUsize::new(0),
        exit: Mutex::new(None),
        unknown: AtomicBool::new(false),
        known,
        blocked,
        gate: (Mutex::new((false, false)), Condvar::new()),
        kills: AtomicUsize::new(0),
    });
    (
        path,
        fake,
        request,
        ModelResources {
            pins: vec![pin],
            root,
            lifetime: vec![],
        },
    )
}
fn endpoint(known: bool, blocked: bool) -> (std::path::PathBuf, Arc<Fake>, ModelEndpoint) {
    let (path, fake, request, resources) = fixture(known, blocked);
    let ep = ModelEndpoint::start(
        Scope::new(
            fake.clone(),
            request,
            "run".into(),
            "b".repeat(64),
            "qa-model".into(),
            Arc::new(|| true),
            resources,
        ),
        "a".repeat(32),
    )
    .unwrap();
    (path, fake, ep)
}
fn request(model: &str) -> ModelRequest {
    ModelRequest {
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        body: serde_json::to_vec(&serde_json::json!({"model":model})).unwrap(),
    }
}
#[test]
fn actor_known_scope_releases_pins_before_private_root() {
    let (path, _, ep) = endpoint(true, false);
    let mut out = Vec::new();
    ep.respond(request("qa-model"), &mut out)
        .unwrap_or_else(|_| panic!("request"));
    assert!(out.windows(9).any(|v| v == b"GENT-UNIT"));
    assert!(out.ends_with(b"0\r\n\r\n"));
    assert!(ep.revoke().is_ok());
    drop(ep);
    let deadline = Instant::now() + Duration::from_secs(2);
    while path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!path.exists());
}
#[test]
fn actor_model_mismatch_writes_no_request() {
    let (_, fake, ep) = endpoint(true, false);
    assert_eq!(
        ep.respond(request("other"), &mut Vec::new())
            .err()
            .unwrap()
            .error,
        ModelError::Request
    );
    assert_eq!(fake.writes.load(Ordering::Acquire), 1);
    assert!(ep.current());
    ep.revoke().unwrap();
}
#[test]
fn actor_sink_failure_unknown_retains_root_and_pins() {
    struct Bad;
    impl Write for Bad {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (path, _, ep) = endpoint(false, false);
    assert!(ep.respond(request("qa-model"), &mut Bad).is_err());
    drop(ep);
    std::thread::sleep(Duration::from_millis(50));
    assert!(path.exists());
    #[cfg(windows)]
    assert!(std::fs::remove_file(path.join("worker.exe")).is_err());
}
#[test]
fn actor_cancel_blocked_write_forces_unknown_without_waiting_stdin_mutex() {
    let (path, fake, ep) = endpoint(true, true);
    let call = ep.clone();
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let result = call.respond(request("qa-model"), &mut Vec::new());
        tx.send(result.is_err()).unwrap();
    });
    let g = fake.gate.0.lock().unwrap();
    let (g, wait) = fake
        .gate
        .1
        .wait_timeout_while(g, Duration::from_secs(2), |v| !v.0)
        .unwrap();
    assert!(!wait.timed_out() && g.0);
    drop(g);
    assert_eq!(ep.revoke(), Err(ModelError::Unknown));
    assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap());
    thread.join().unwrap();
    assert_eq!(fake.kills.load(Ordering::Acquire), 1);
    drop(ep);
    assert!(path.exists());
}
#[test]
fn actor_revoke_before_bootstrap_writes_nothing() {
    let (path, fake, request, resources) = fixture(true, false);
    assert!(
        ModelEndpoint::start(
            Scope::new(
                fake.clone(),
                request,
                "run".into(),
                "b".repeat(64),
                "qa-model".into(),
                Arc::new(|| false),
                resources
            ),
            "a".repeat(32)
        )
        .is_err()
    );
    assert_eq!(fake.writes.load(Ordering::Acquire), 0);
    assert!(!path.exists());
}
#[test]
fn actor_launch_error_conservatively_retains_prepared_resources() {
    let (path, _, _, resources) = fixture(true, false);
    scope::retain_launch_error(resources);
    assert!(path.exists());
}
