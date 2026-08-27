use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::interrupt::{
    InterruptPolicy, ProcessTreeControl, ProcessTreeError, ProcessTreeSignal,
};
use gent_drivers::lock::capture;
use gent_drivers::{
    DriverRunRunner, LaunchIntent, OutputLimits, ProcessLauncher, ProviderLaunch, ProviderProcess,
    SessionEffect, SupervisorError,
};
use gent_ports::{PublicProviderRunError, PublicProviderRunner};
use gent_testkit::{FakeProcess, FakeProcessSignal};
use gent_types::RunVersionLock;

#[derive(Debug)]
struct FakeTree {
    process: Arc<FakeProcess>,
    exit_state: Arc<Mutex<ExitState>>,
}

#[derive(Clone, Copy, Debug)]
enum ExitState {
    Active,
    Exited(Option<i32>),
}

impl ProcessTreeControl for FakeTree {
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        self.process.signal(match signal {
            ProcessTreeSignal::Interrupt => FakeProcessSignal::Interrupt,
            ProcessTreeSignal::Terminate => FakeProcessSignal::Terminate,
            ProcessTreeSignal::Kill => FakeProcessSignal::Kill,
        });
        Ok(())
    }
}

impl ProviderProcess for FakeTree {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }

    fn close_stdin(&self) -> Result<(), ProcessTreeError> {
        Ok(())
    }

    fn next_stdout_chunk(&self) -> Result<Option<Vec<u8>>, ProcessTreeError> {
        Ok(self.process.read_stdout())
    }

    fn try_exit_code(&self) -> Result<Option<Option<i32>>, ProcessTreeError> {
        Ok(match *self.exit_state.lock().unwrap() {
            ExitState::Active => None,
            ExitState::Exited(code) => Some(code),
        })
    }
}

#[derive(Debug)]
struct FakeLauncher {
    process: Arc<FakeProcess>,
    launches: Arc<Mutex<Vec<ProviderLaunch>>>,
    exit_state: Arc<Mutex<ExitState>>,
    fail: bool,
}

impl FakeLauncher {
    fn new(process: Arc<FakeProcess>, launches: Arc<Mutex<Vec<ProviderLaunch>>>) -> Self {
        Self {
            process,
            launches,
            exit_state: Arc::new(Mutex::new(ExitState::Active)),
            fail: false,
        }
    }

    fn exit(&self, code: Option<i32>) {
        *self.exit_state.lock().unwrap() = ExitState::Exited(code);
    }
}

impl ProcessLauncher for FakeLauncher {
    type Process = FakeTree;

    fn launch(&self, launch: &ProviderLaunch) -> Result<FakeTree, SupervisorError> {
        if self.fail {
            return Err(SupervisorError::Launch("scripted failure".into()));
        }
        self.launches.lock().unwrap().push(launch.clone());
        Ok(FakeTree {
            process: Arc::clone(&self.process),
            exit_state: Arc::clone(&self.exit_state),
        })
    }
}

fn lock(path: &Path) -> RunVersionLock {
    capture("claude", path, "1", "test").unwrap()
}

fn runner(launcher: FakeLauncher) -> DriverRunRunner<FakeLauncher, FakeTree> {
    DriverRunRunner::new(
        launcher,
        OutputLimits::new(16, 32),
        BufferPolicy::new(2, 64, 0, 0).unwrap(),
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
    assert_eq!(
        runner.write_frame("missing", b"frame\n"),
        Err(PublicProviderRunError::NotActive)
    );
    runner.write_frame("start", b"frame\n").unwrap();
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
    assert_eq!(
        launches[0].arguments,
        [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--print",
            "--verbose",
        ]
    );
    assert!(matches!(
        &launches[1].intent,
        LaunchIntent::Resume { session_id } if session_id == "session-a"
    ));
    assert_eq!(
        launches[1].arguments,
        [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--print",
            "--verbose",
            "--resume",
            "session-a",
        ]
    );
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
        exit_state: Arc::new(Mutex::new(ExitState::Active)),
        fail: true,
    };
    assert!(matches!(
        runner(launcher).start("failed", &run_lock),
        Err(PublicProviderRunError::Failed(message)) if message.contains("scripted failure")
    ));
}

#[test]
fn interrupt_deadlines_escalate_and_exit_cancels_the_remaining_ladder() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "public").unwrap();
    let process = Arc::new(FakeProcess::default());
    let runner = runner(FakeLauncher::new(
        Arc::clone(&process),
        Arc::new(Mutex::new(Vec::new())),
    ));
    let run_lock = lock(&executable);
    runner.start("escalate", &run_lock).unwrap();

    let start = Instant::now();
    runner.interrupt_at("escalate", start).unwrap();
    assert!(
        !runner
            .advance_interrupt("escalate", start + Duration::from_millis(0))
            .unwrap()
    );
    assert!(
        runner
            .advance_interrupt("escalate", start + Duration::from_millis(1))
            .unwrap()
    );
    assert!(
        runner
            .advance_interrupt("escalate", start + Duration::from_millis(2))
            .unwrap()
    );
    assert_eq!(
        process.signals(),
        [
            FakeProcessSignal::Interrupt,
            FakeProcessSignal::Terminate,
            FakeProcessSignal::Kill
        ]
    );

    runner.start("exit", &run_lock).unwrap();
    runner.interrupt_at("exit", start).unwrap();
    let signals_before_exit = process.signals();
    let _ = runner.process_exited("exit", Some(0)).unwrap();
    assert_eq!(
        runner.advance_interrupt("exit", start + Duration::from_secs(1)),
        Err(PublicProviderRunError::NotActive)
    );
    assert_eq!(process.signals(), signals_before_exit);
}

#[test]
fn runner_drains_stdout_before_settling_and_releasing_an_exited_process() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    fs::write(&executable, "public").unwrap();
    let process = Arc::new(FakeProcess::default());
    let launcher = FakeLauncher::new(Arc::clone(&process), Arc::new(Mutex::new(Vec::new())));
    let runner = runner(FakeLauncher {
        process: Arc::clone(&launcher.process),
        launches: Arc::clone(&launcher.launches),
        exit_state: Arc::clone(&launcher.exit_state),
        fail: false,
    });
    runner.start("exit-drain", &lock(&executable)).unwrap();
    process.push_stdout(
        br#"{"type":"session_started","session_id":"s"}
"#,
    );
    let _ = runner.poll_stdout("exit-drain").unwrap();
    process.push_stdout(
        br#"{"type":"output","text":"last"}
"#,
    );
    launcher.exit(Some(0));

    assert!(matches!(
        runner.poll_stdout("exit-drain").unwrap().as_deref(),
        Some([SessionEffect::Normalized { .. }])
    ));
    assert_eq!(
        runner.poll_stdout("exit-drain").unwrap(),
        Some(vec![SessionEffect::Terminal {
            reason: "providerExited:0".into()
        }])
    );
    assert_eq!(
        runner.poll_stdout("exit-drain"),
        Err(PublicProviderRunError::NotActive)
    );
}
