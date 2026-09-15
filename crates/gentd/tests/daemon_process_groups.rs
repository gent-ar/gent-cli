#![cfg(unix)]

use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use gent_drivers::process::{configure_process_tree, groups::ProcessGroups};

struct Gentd(Child);

impl Drop for Gentd {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(data_dir: &Path) -> Gentd {
    let empty_path = data_dir.parent().unwrap().join("empty-path");
    std::fs::create_dir_all(&empty_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gentd"))
        .arg("--standalone-authority")
        .arg("--data-dir")
        .arg(data_dir)
        .env("PATH", empty_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let socket = data_dir.join("gentd.sock");
    let deadline = Instant::now() + Duration::from_secs(20);
    while UnixStream::connect(&socket).is_err() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("gentd exited before serving: {status}");
        }
        assert!(Instant::now() < deadline, "gentd did not serve");
        std::thread::sleep(Duration::from_millis(25));
    }
    Gentd(child)
}

fn provider_group() -> (Child, u32) {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "sleep 60 & echo $!; while :; do sleep 1; done"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_tree(&mut command);
    let mut leader = command.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(leader.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    (leader, line.trim().parse().unwrap())
}

fn alive(pid: u32) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8(output.stdout).unwrap();
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

fn eventually(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn exited(gentd: &mut Gentd) -> Option<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = gentd.0.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

fn record(data_dir: &Path, pid: u32) {
    ProcessGroups::open(&data_dir.join("process-groups"))
        .unwrap()
        .record(pid)
        .unwrap();
}

fn stop(mut leader: Child) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{}", leader.id())])
        .stderr(Stdio::null())
        .status();
    let _ = leader.wait();
}

#[test]
fn a_provider_group_left_by_a_sigkilled_gentd_is_stopped_when_gentd_starts_again() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let mut first = start(&data_dir);
    let (leader, grandchild) = provider_group();
    record(&data_dir, leader.id());

    first.0.kill().unwrap();
    first.0.wait().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(alive(grandchild), "SIGKILL cannot reap provider children");

    let _second = start(&data_dir);

    let stopped = eventually(|| !alive(grandchild));
    let records_left = std::fs::read_dir(data_dir.join("process-groups"))
        .unwrap()
        .count();
    stop(leader);
    assert!(
        stopped,
        "the restarted gentd stopped the orphaned provider group"
    );
    assert_eq!(records_left, 0);
}

#[test]
fn sigterm_stops_recorded_provider_groups_before_gentd_exits() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let mut gentd = start(&data_dir);
    let (leader, grandchild) = provider_group();
    record(&data_dir, leader.id());

    Command::new("/bin/kill")
        .args(["-TERM", &gentd.0.id().to_string()])
        .status()
        .unwrap();

    let status = exited(&mut gentd);
    let stopped = eventually(|| !alive(grandchild));
    stop(leader);
    assert!(status.is_some_and(|status| status.success()));
    assert!(stopped, "a terminated gentd stopped its provider group");
}
