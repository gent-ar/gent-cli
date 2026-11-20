use super::*;
use std::thread;
use std::time::Duration;
#[test]
fn owned_os_exit_peek_does_not_join_a_live_descendant_pipe() {
    const ROLE: &str = "GENT_OWNED_EXIT_PEEK_FIXTURE";
    let exe = std::env::current_exe().unwrap();
    let test_name = "process::sandbox_pipe_lifetime_tests::owned_os_exit_peek_does_not_join_a_live_descendant_pipe";
    match std::env::var(ROLE).ok().as_deref() {
        Some("descendant") => {
            thread::sleep(Duration::from_secs(3));
            return;
        }
        Some("owner") => {
            let _child = Command::new(exe)
                .args(["--exact", test_name, "--nocapture"])
                .env(ROLE, "descendant")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            std::process::exit(126);
        }
        _ => {}
    }
    let mut child = Command::new(exe)
        .args(["--exact", test_name, "--nocapture"])
        .env(ROLE, "owner")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let process = SystemProcess::new(child, stdin, stdout, stderr, 4096).unwrap();
    let started = std::time::Instant::now();
    let mut observed = None;
    while started.elapsed() < Duration::from_secs(2) {
        observed = process.peek_owned_exit_status().unwrap();
        if observed.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(observed.and_then(|status| status.code()), Some(126));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "OS-exit observation must not join inherited pipes"
    );
    process.wait().unwrap();
}

#[test]
fn exit_is_decided_while_a_grandchild_still_holds_the_inherited_stdout() {
    const ROLE: &str = "GENT_EXIT_WITH_PIPE_HOLDER_FIXTURE";
    let exe = std::env::current_exe().unwrap();
    let test_name = "process::sandbox_pipe_lifetime_tests::exit_is_decided_while_a_grandchild_still_holds_the_inherited_stdout";
    match std::env::var(ROLE).ok().as_deref() {
        Some("holder") => {
            thread::sleep(Duration::from_secs(30));
            return;
        }
        Some("parent") => {
            let holder = Command::new(&exe)
                .args(["--exact", test_name, "--nocapture"])
                .env(ROLE, "holder")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            println!("holder-pid={}", holder.id());
            std::process::exit(0);
        }
        _ => {}
    }
    let mut child = Command::new(&exe)
        .args(["--exact", test_name, "--nocapture"])
        .env(ROLE, "parent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let process = SystemProcess::new(child, stdin, stdout, stderr, 64 * 1024).unwrap();
    let started = std::time::Instant::now();
    let mut seen = Vec::new();
    let mut exit = None;
    let holder = loop {
        while let Some(chunk) = process.next_stdout_chunk().unwrap() {
            seen.extend(chunk);
        }
        exit = exit.or(process.try_exit_code().unwrap());
        let holder = String::from_utf8_lossy(&seen)
            .split("holder-pid=")
            .nth(1)
            .and_then(|rest| rest.lines().next()?.trim().parse::<u32>().ok());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "exit {exit:?} after {:?}",
            started.elapsed()
        );
        if let (Some(_), Some(holder)) = (exit, holder) {
            break holder;
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(exit, Some(Some(0)));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while windows::image_name(holder).is_some() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(windows::image_name(holder), None);
}

#[test]
fn dropping_a_provider_kills_descendants_that_outlived_their_parent() {
    let mut child = gent_types::hidden_command("cmd")
        .args([
            "/C",
            "ping -n 2 127.0.0.1 >NUL & start /B ping -n 120 127.0.0.1 >NUL",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let process = SystemProcess::new(child, stdin, stdout, stderr, 4096).unwrap();
    let tree = process.tree.lock().unwrap();
    let job = tree.as_ref().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let orphan = loop {
        let parent_exited = matches!(process.peek_owned_exit_status(), Ok(Some(_)));
        let pids = job.query_process_id_list().unwrap();
        let images = pids
            .iter()
            .filter_map(|pid| u32::try_from(*pid).ok())
            .map(|pid| (pid, windows::image_name(pid)))
            .collect::<Vec<_>>();
        if parent_exited
            && let Some((pid, _)) = images
                .iter()
                .find(|(_, image)| image.as_deref() == Some("ping.exe"))
        {
            break *pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "parent exited: {parent_exited}, job: {images:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(windows::image_name(orphan).as_deref(), Some("ping.exe"));

    drop(tree);
    drop(process);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while windows::image_name(orphan).is_some() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(windows::image_name(orphan), None);
}
