use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::interrupt::{
    InterruptPolicy, ProcessTreeControl, ProcessTreeError, ProcessTreeSignal,
};
use gent_drivers::lock::capture;
use gent_drivers::{
    DriverRunRunner, LaunchIntent, OutputLimits, ProcessLauncher, ProviderLaunch, SupervisorError,
};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_testkit::{FakeProcess, FakeProcessSignal};
use gent_types::RunVersionLock;

#[derive(Debug)]
struct FakeTree(Arc<FakeProcess>);

impl ProcessTreeControl for FakeTree {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.0.signal(match signal {
            ProcessTreeSignal::Interrupt => FakeProcessSignal::Interrupt,
            ProcessTreeSignal::Terminate => FakeProcessSignal::Terminate,
            ProcessTreeSignal::Kill => FakeProcessSignal::Kill,
        });
        Ok(())
    }
}

#[derive(Debug)]
struct FakeLauncher {
    process: Arc<FakeProcess>,
    launches: Arc<Mutex<Vec<ProviderLaunch>>>,
    fail: bool,
}

impl FakeLauncher {
    fn new(process: Arc<FakeProcess>, launches: Arc<Mutex<Vec<ProviderLaunch>>>) -> Self {
        Self {
            process,
            launches,
            fail: false,
        }
    }
}

impl ProcessLauncher for FakeLauncher {
    type Process = FakeTree;

    fn launch(&self, launch: &ProviderLaunch) -> Result<FakeTree, SupervisorError> {
        if self.fail {
            return Err(SupervisorError::Launch("scripted failure".into()));
        }
        self.launches.lock().unwrap().push(launch.clone());
        Ok(FakeTree(Arc::clone(&self.process)))
    }
}

fn lock(path: &Path) -> RunVersionLock {
    capture("claude", path, "1", "test").unwrap()
}

fn runner(launcher: FakeLauncher) -> DriverRunRunner<FakeLauncher, FakeTree> {
    DriverRunRunner::new(
        launcher,
        OutputLimits::new(16, 32),
        BufferPolicy::new(2, 32, 0, 0).unwrap(),
        InterruptPolicy {
            interrupt_grace_ms: 1,
            terminate_grace_ms: 1,
        },
    )
}

#[test]
fn starts_resumes_rejects_duplicates_and_interrupts_only_active_runs() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "public").unwrap();
    let process = Arc::new(FakeProcess::default());
    let launches = Arc::new(Mutex::new(Vec::new()));
    let runner = runner(FakeLauncher::new(
        Arc::clone(&process),
        Arc::clone(&launches),
    ));
    let run_lock = lock(&executable);

    runner.start("start", &run_lock).unwrap();
    runner.resume("resume", &run_lock, "session-a").unwrap();
    assert!(matches!(
        runner.start("start", &run_lock),
        Err(PublicProviderRunError::Failed(message)) if message == "run already active"
    ));
    assert!(matches!(
        runner.resume("resume", &run_lock, "session-b"),
        Err(PublicProviderRunError::Failed(message)) if message == "run already active"
    ));
    assert_eq!(
        runner.interrupt("missing"),
        Err(PublicProviderRunError::NotActive)
    );
    runner.interrupt("start").unwrap();
    assert_eq!(process.signals(), vec![FakeProcessSignal::Interrupt]);

    let launches = launches.lock().unwrap();
    assert!(matches!(launches[0].intent, LaunchIntent::Start));
    assert!(matches!(
        &launches[1].intent,
        LaunchIntent::Resume { session_id } if session_id == "session-a"
    ));
}

#[test]
fn changed_locks_and_launcher_failures_are_never_silently_accepted() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "before").unwrap();
    let run_lock = lock(&executable);
    fs::write(&executable, "after").unwrap();
    let changed_runner = runner(FakeLauncher::new(
        Arc::new(FakeProcess::default()),
        Arc::new(Mutex::new(Vec::new())),
    ));
    assert_eq!(
        changed_runner.start("changed", &run_lock),
        Err(PublicProviderRunError::ProviderChanged)
    );

    fs::write(&executable, "stable").unwrap();
    let run_lock = lock(&executable);
    let launcher = FakeLauncher {
        process: Arc::new(FakeProcess::default()),
        launches: Arc::new(Mutex::new(Vec::new())),
        fail: true,
    };
    assert!(matches!(
        runner(launcher).start("failed", &run_lock),
        Err(PublicProviderRunError::Failed(message)) if message.contains("scripted failure")
    ));
}
