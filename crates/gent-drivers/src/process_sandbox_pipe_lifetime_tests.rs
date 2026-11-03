use super::*;
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
    let process = SystemProcess::new(child, stdin, stdout, stderr, 4096);
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
    // Only this fixture knows that its owned descendant exits naturally after 3s.
    // A real unknown sandbox is quarantined instead of waiting/joining here.
    process.wait().unwrap();
}
