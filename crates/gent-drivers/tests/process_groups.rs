#![cfg(unix)]

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use gent_drivers::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, SystemLauncher,
    process::{
        configure_process_tree,
        groups::{self, ProcessGroups, StopRecordedOnExit},
    },
};

fn state(pid: u32) -> String {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn process_group(pid: u32) -> u32 {
    let output = Command::new("/bin/ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn alive(pid: u32) -> bool {
    let state = state(pid);
    !state.is_empty() && !state.starts_with('Z')
}

fn eventually_dead(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn provider_group(script: &str) -> (Child, u32) {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_process_tree(&mut command);
    let mut child = command.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let grandchild = line.trim().parse().unwrap();
    (child, grandchild)
}

fn kill_group(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

fn ledger() -> (tempfile::TempDir, ProcessGroups) {
    let directory = tempfile::tempdir().unwrap();
    let groups = ProcessGroups::open(&directory.path().join("process-groups")).unwrap();
    (directory, groups)
}

fn records(groups_dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(groups_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn installed_directory() -> &'static PathBuf {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep().join("process-groups");
        std::mem::forget(groups::install(&directory).unwrap().0);
        directory
    })
}

fn shell_launch(script: &str) -> ProviderLaunch {
    let lock =
        gent_drivers::lock::capture("claude", std::path::Path::new("/bin/sh"), "test", "test")
            .unwrap();
    ProviderLaunch {
        executable: lock.canonical_path.clone().into(),
        lock,
        provider: "claude".into(),
        arguments: vec!["-c".into(), script.into()],
        intent: LaunchIntent::Start,
        workspace_root: None,
        workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
    }
}

fn first_line(process: &gent_drivers::process::SystemProcess) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = Vec::new();
    while !bytes.contains(&b'\n') {
        assert!(Instant::now() < deadline, "provider printed nothing");
        match process.next_stdout_chunk().unwrap() {
            Some(chunk) => bytes.extend(chunk),
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn dropping_a_provider_that_ignores_its_input_kills_its_whole_process_group() {
    let process = SystemLauncher::new(1024)
        .launch(&shell_launch(
            "sleep 60 & echo $!; while :; do sleep 1; done",
        ))
        .unwrap();
    let grandchild: u32 = first_line(&process).parse().unwrap();
    assert!(alive(grandchild));

    drop(process);

    assert!(eventually_dead(grandchild));
}

const ESCAPE_SESSION: &str =
    "perl -MPOSIX -e 'POSIX::setsid(); exec \"/bin/sh\", \"-c\", \"sleep 60 & echo \\$!; wait\"' &";

fn escaped_session_script(tail: &str) -> String {
    format!("{ESCAPE_SESSION} {tail}")
}

#[test]
fn dropping_a_provider_kills_descendants_that_left_its_process_group() {
    let process = SystemLauncher::new(1024)
        .launch(&shell_launch(&escaped_session_script(
            "while :; do sleep 1; done",
        )))
        .unwrap();
    let escaped: u32 = first_line(&process).parse().unwrap();
    assert!(alive(escaped));
    assert_ne!(process_group(escaped), process_group(std::process::id()));

    drop(process);

    assert!(eventually_dead(escaped));
}

#[test]
fn a_group_left_by_a_killed_daemon_is_stopped_with_its_escaped_descendants() {
    let (directory, groups) = ledger();
    let (mut leader, escaped) = provider_group(&escaped_session_script("wait"));
    groups.record(leader.id()).unwrap();
    assert_ne!(process_group(escaped), leader.id());

    let restarted = ProcessGroups::open(&directory.path().join("process-groups")).unwrap();
    assert_eq!(restarted.stop_recorded().unwrap(), vec![leader.id()]);

    assert!(eventually_dead(escaped));
    leader.wait().unwrap();
}

#[test]
fn a_launched_provider_is_recorded_until_it_is_reaped() {
    let directory = installed_directory();
    let process = SystemLauncher::new(1024)
        .launch(&shell_launch("echo $$; read line"))
        .unwrap();
    let leader = first_line(&process);
    assert!(records(directory).contains(&leader));

    process.close_stdin().unwrap();
    process.wait().unwrap();

    assert!(!records(directory).contains(&leader));
}

#[test]
fn a_group_left_by_a_killed_daemon_is_stopped_on_the_next_start() {
    let (directory, groups) = ledger();
    let (mut leader, grandchild) = provider_group("sleep 60 & echo $!; wait");
    groups.record(leader.id()).unwrap();

    let restarted = ProcessGroups::open(&directory.path().join("process-groups")).unwrap();
    assert_eq!(restarted.stop_recorded().unwrap(), vec![leader.id()]);

    assert!(eventually_dead(grandchild));
    leader.wait().unwrap();
    assert!(records(&directory.path().join("process-groups")).is_empty());
}

#[test]
fn a_group_whose_leader_already_exited_is_still_stopped() {
    let (directory, groups) = ledger();
    let (mut leader, grandchild) = provider_group("sleep 60 & echo $!");
    groups.record(leader.id()).unwrap();
    leader.wait().unwrap();
    assert!(alive(grandchild));

    assert_eq!(groups.stop_recorded().unwrap(), vec![leader.id()]);

    assert!(eventually_dead(grandchild));
    assert!(records(&directory.path().join("process-groups")).is_empty());
}

#[test]
fn a_record_whose_pid_now_names_another_process_is_discarded_without_killing_it() {
    let (directory, groups) = ledger();
    let (mut unrelated, grandchild) = provider_group("sleep 60 & echo $!; wait");
    std::fs::write(
        directory
            .path()
            .join("process-groups")
            .join(unrelated.id().to_string()),
        "a process started before this one existed",
    )
    .unwrap();

    assert!(groups.stop_recorded().unwrap().is_empty());

    assert!(alive(unrelated.id()));
    assert!(alive(grandchild));
    assert!(records(&directory.path().join("process-groups")).is_empty());
    kill_group(unrelated.id());
    unrelated.wait().unwrap();
}

#[test]
fn a_panicking_daemon_stops_its_recorded_groups_while_unwinding() {
    let (_directory, groups) = ledger();
    let (mut leader, grandchild) = provider_group("sleep 60 & echo $!; wait");
    groups.record(leader.id()).unwrap();

    let unwound = std::panic::catch_unwind(|| {
        let _stop = StopRecordedOnExit::new(groups.clone());
        panic!("the daemon panicked");
    });

    assert!(unwound.is_err());
    assert!(eventually_dead(grandchild));
    leader.wait().unwrap();
}

#[test]
fn released_provider_processes_are_reaped_instead_of_left_as_zombies() {
    let processes = (0..8)
        .map(|_| {
            SystemLauncher::new(64)
                .launch(&shell_launch("echo $$"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let leaders = processes
        .iter()
        .map(|process| first_line(process).parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !leaders.iter().all(|pid| state(*pid).starts_with('Z')) {
        assert!(Instant::now() < deadline, "providers did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }

    drop(processes);

    let zombies = leaders
        .iter()
        .filter(|pid| state(**pid).starts_with('Z'))
        .count();
    assert_eq!(zombies, 0);
}
