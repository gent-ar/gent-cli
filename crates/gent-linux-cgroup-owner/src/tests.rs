use super::*;
#[test]
fn finite_public_scope_names_only() {
    assert!(nonce_ok(&"a".repeat(32)));
    for bad in ["../other", "gent", "", &"A".repeat(32), &"a".repeat(33)] {
        assert!(!nonce_ok(bad));
    }
}
#[test]
fn resource_limits_reject_zero_unaligned_and_overflow() {
    let good = SandboxResourceLimits {
        max_processes: 16,
        max_memory_bytes: 64 * 1024 * 1024,
        max_cpu_time_ms: 5000,
    };
    assert_eq!(limits_budget(good), Ok(5_000_000));
    assert_eq!(
        limits_budget(SandboxResourceLimits {
            max_processes: 0,
            ..good
        }),
        Err(Error::InvalidLimits)
    );
    assert_eq!(
        limits_budget(SandboxResourceLimits {
            max_memory_bytes: 4097,
            ..good
        }),
        Err(Error::InvalidLimits)
    );
    assert_eq!(
        limits_budget(SandboxResourceLimits {
            max_cpu_time_ms: u64::MAX,
            ..good
        }),
        Err(Error::InvalidLimits)
    );
}
#[test]
fn kernel_counters_are_bounded_and_unambiguous() {
    assert_eq!(
        field("usage_usec 123\nuser_usec 100\n", "usage_usec"),
        Ok(123)
    );
    for bad in [
        "usage_usec 1\nusage_usec 2",
        "usage_usec NaN",
        "usage_usec 1 trailing",
        "other 1",
        "",
    ] {
        assert_eq!(field(bad, "usage_usec"), Err(Error::MalformedState));
    }
}
#[test]
fn ordinary_directory_cannot_fake_delegation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("cgroup.controllers"), "memory pids cpu").unwrap();
    let Err(error) = DelegatedParent::inspect(root.path()) else {
        panic!("ordinary tree admitted")
    };
    assert!(matches!(
        error,
        Error::InvalidPath | Error::PrivilegedCaller
    ));
    assert_eq!(
        std::fs::read_to_string(root.path().join("cgroup.controllers")).unwrap(),
        "memory pids cpu"
    );
}
#[test]
fn traversal_is_rejected_before_any_write() {
    let Err(error) = DelegatedParent::inspect(Path::new("/sys/fs/cgroup/../fs")) else {
        panic!("traversal admitted")
    };
    assert!(matches!(
        error,
        Error::InvalidPath | Error::PrivilegedCaller
    ));
}
#[test]
#[ignore = "requires explicit UID65534 host-negative runner"]
fn actual_host_has_no_unprivileged_delegation_and_is_unchanged() {
    assert_eq!(
        rustix::process::geteuid().as_raw(),
        65534,
        "run this host-specific test via owned unprivileged runner"
    );
    let path = Path::new("/sys/fs/cgroup");
    let before = std::fs::read_to_string(path.join("cgroup.subtree_control")).unwrap();
    let names = || {
        let mut n = std::fs::read_dir(path)
            .unwrap()
            .map(|p| p.unwrap().file_name())
            .filter(|p| p.to_string_lossy().starts_with("gent-"))
            .collect::<Vec<_>>();
        n.sort();
        n
    };
    let initial = names();
    assert!(matches!(
        DelegatedParent::inspect(path),
        Err(Error::NotDelegated)
    ));
    assert_eq!(
        std::fs::read_to_string(path.join("cgroup.subtree_control")).unwrap(),
        before
    );
    assert_eq!(names(), initial);
}

fn child(code: &str) -> Child {
    std::process::Command::new("/usr/bin/python3")
        .args(["-u", "-c", code])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
fn birth(child: &Child) -> String {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned()
}
fn scope(parent: &Path, pids: u16, memory: u64, cpu_ms: u64) -> CgroupScope {
    let nonce = format!(
        "{:032x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    CgroupScope::create(
        DelegatedParent::inspect(parent).unwrap(),
        &nonce,
        SandboxResourceLimits {
            max_processes: pids,
            max_memory_bytes: memory,
            max_cpu_time_ms: cpu_ms,
        },
    )
    .unwrap()
}
fn start(scope: &CgroupScope, child: &mut Child) {
    let generation = birth(child);
    scope.attach_blocked_helper(child, &generation).unwrap();
    child.stdin.take().unwrap().write_all(b"GO\n").unwrap();
}
fn wait(child: &mut Child, scope: &mut CgroupScope) -> std::process::ExitStatus {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > until {
            let _ = scope.terminate_and_remove(Duration::from_secs(2));
            panic!("bounded helper deadline");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
#[ignore = "requires explicit pre-existing or scoped QA delegation, never auto-created by library"]
fn actual_scoped_kernel_limits_and_cleanup() {
    let parent = std::path::PathBuf::from(std::env::var("GENT_CGROUP_QA_PARENT").unwrap());
    assert_eq!(uid().unwrap(), 59991);
    // Positive controls run in our bounded controller leaf, outside worker limits.
    let positive=std::process::Command::new("/usr/bin/python3").args(["-c","import subprocess; p=[subprocess.Popen(['/bin/sleep','5']) for _ in range(6)]; [x.kill() for x in p]; [x.wait() for x in p]; x=bytearray(64*1024*1024); print('positive-controls')"]).output().unwrap();
    assert!(positive.status.success());
    assert_eq!(positive.stdout, b"positive-controls\n");
    let mut pids = scope(&parent, 4, 64 * 1024 * 1024, 5000);
    let mut helper = child(
        "import sys,subprocess,errno;sys.stdin.readline();p=[]\ntry:\n for _ in range(12):p.append(subprocess.Popen(['/bin/sleep','20']))\nexcept OSError as e: print('denied='+str(e.errno),flush=True)\nfinally:\n for x in p:x.kill()\n for x in p:x.wait()\n",
    );
    start(&pids, &mut helper);
    assert!(wait(&mut helper, &mut pids).success());
    let pids_denials = field(&read(&pids.fd, "pids.events").unwrap(), "max").unwrap();
    assert!(pids_denials > 0);
    let done = pids.terminate_and_remove(Duration::from_secs(2)).unwrap();
    assert!(done.empty && done.removed);
    let mut output = String::new();
    helper
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert_eq!(output, "denied=11\n");
    println!(
        "GENT-CGROUP-FACT {{\"case\":\"pids\",\"kernelDenials\":{pids_denials},\"cleanup\":true}}"
    );
    let mut memory = scope(&parent, 8, 32 * 1024 * 1024, 5000);
    let mut helper = child(
        "import sys;sys.stdin.readline();x=bytearray(96*1024*1024);print('unexpected-allocation')",
    );
    start(&memory, &mut helper);
    assert!(!wait(&mut helper, &mut memory).success());
    let oom_kills = field(&read(&memory.fd, "memory.events").unwrap(), "oom_kill").unwrap();
    assert!(oom_kills > 0);
    assert!(
        memory
            .terminate_and_remove(Duration::from_secs(2))
            .unwrap()
            .removed
    );
    println!("GENT-CGROUP-FACT {{\"case\":\"memory\",\"oomKills\":{oom_kills},\"cleanup\":true}}");
    let mut cpu = scope(&parent, 8, 64 * 1024 * 1024, 50);
    let mut helper = child(
        "import sys,subprocess;sys.stdin.readline();subprocess.Popen([sys.executable,'-c','while True:pass'])\nwhile True:pass",
    );
    start(&cpu, &mut helper);
    let until = Instant::now() + Duration::from_secs(3);
    let observed = loop {
        let sample = cpu.sample_cpu_budget().unwrap();
        if sample.exceeded {
            break sample;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(!wait(&mut helper, &mut cpu).success());
    let until = Instant::now() + Duration::from_secs(2);
    while field(&read(&cpu.fd, "cgroup.events").unwrap(), "populated").unwrap() != 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    let final_usage = field(&read(&cpu.fd, "cpu.stat").unwrap(), "usage_usec").unwrap();
    assert!(
        cpu.terminate_and_remove(Duration::from_secs(2))
            .unwrap()
            .removed
    );
    println!(
        "GENT-CGROUP-FACT {{\"case\":\"cpu-monitored\",\"budgetUsec\":{},\"sampleUsec\":{},\"finalUsec\":{final_usage},\"hardCeilingClaim\":false,\"cleanup\":true}}",
        observed.budget_usec, observed.usage_usec
    );
    // Trusted helper enters the scope while blocked, then execs the real typed SDK fixture.
    let binary = std::env::var("GENT_MXC_TYPED_TEST_BINARY").unwrap();
    assert!(Path::new(&binary).is_absolute());
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("home")).unwrap();
    let mut sdk = scope(&parent, 32, 128 * 1024 * 1024, 10000);
    let code = format!(
        "import os,sys;sys.stdin.readline();os.execv({binary:?},[{binary:?},'engine_tests::actual_typed_engine_runs_deny_network_fixture','--exact','--ignored','--test-threads=1'])"
    );
    let mut helper = std::process::Command::new("/usr/bin/python3")
        .args(["-u", "-c", &code])
        .env("GENT_MXC_TEST_WORKSPACE", workspace.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    start(&sdk, &mut helper);
    assert!(wait(&mut helper, &mut sdk).success());
    let peak = read(&sdk.fd, "pids.peak")
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert!(peak >= 2);
    assert!(
        sdk.terminate_and_remove(Duration::from_secs(2))
            .unwrap()
            .removed
    );
    let mut output = String::new();
    helper
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(output.contains("1 passed; 0 failed"));
    println!(
        "GENT-CGROUP-FACT {{\"case\":\"typed-mxc-inherited-scope\",\"peakTasks\":{peak},\"cleanup\":true}}"
    );
}

#[test]
fn namespace_delegation_is_bound_to_actual_open_mount() {
    let good = "7 1 0:1 / /sys/fs/cgroup rw - cgroup2 cgroup rw,nsdelegate\n";
    assert_eq!(namespace_delegated_mount("mnt_id:\t7\n", good), Ok(()));
    assert_eq!(
        namespace_delegated_mount("mnt_id: 8\n", good),
        Err(Error::MalformedState)
    );
    let wrong = "7 1 0:1 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n8 1 0:2 / /other rw - cgroup2 cgroup rw,nsdelegate\n";
    assert_eq!(
        namespace_delegated_mount("mnt_id: 7\n", wrong),
        Err(Error::MissingNamespaceDelegation)
    );
    assert_eq!(
        namespace_delegated_mount("mnt_id: 7\nmnt_id: 7\n", good),
        Err(Error::MalformedState)
    );
    assert_eq!(
        namespace_delegated_mount("mnt_id: 7\n", &(good.to_owned() + good)),
        Err(Error::MalformedState)
    );
    assert_eq!(
        namespace_delegated_mount(
            "mnt_id: 7\n",
            &good.replace("rw,nsdelegate", "rw,nonsdelegate")
        ),
        Err(Error::MissingNamespaceDelegation)
    );
    assert_eq!(
        namespace_delegated_mount(
            "mnt_id: 7\n",
            &good.replace("cgroup2 cgroup", "tmpfs tmpfs")
        ),
        Err(Error::WrongFilesystem)
    );
}
#[test]
fn actual_open_cgroup_mount_namespace_policy_is_read_only() {
    let fd = File::open("/sys/fs/cgroup").unwrap();
    let result = require_namespace_delegation(&fd);
    assert!(matches!(
        result,
        Ok(()) | Err(Error::MissingNamespaceDelegation)
    ));
}
