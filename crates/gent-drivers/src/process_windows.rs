use std::process::ExitStatus;

use crate::interrupt::{ProcessTreeError, ProcessTreeSignal};

pub(super) fn signal_tree(pid: i32, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    signal_with(pid, signal, taskkill, process_exists)
}

fn signal_with<F, A>(
    pid: i32,
    signal: ProcessTreeSignal,
    mut kill: F,
    exists: A,
) -> Result<(), ProcessTreeError>
where
    F: FnMut(i32, bool) -> Result<ExitStatus, ProcessTreeError>,
    A: Fn(i32) -> Result<bool, ProcessTreeError>,
{
    if pid <= 0 {
        return Err(ProcessTreeError::Failed(
            "invalid provider process ID".into(),
        ));
    }
    let first = kill(pid, signal == ProcessTreeSignal::Kill)?;
    if first.success() {
        return Ok(());
    }
    let last = if signal == ProcessTreeSignal::Kill {
        first
    } else {
        kill(pid, true)?
    };
    if last.success() || !exists(pid)? {
        return Ok(());
    }
    Err(ProcessTreeError::Failed(format!(
        "taskkill exited with {last}; provider process {pid} is still running"
    )))
}

fn taskkill(pid: i32, force: bool) -> Result<ExitStatus, ProcessTreeError> {
    let mut command = gent_types::hidden_command("taskkill");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }
    command
        .output()
        .map(|output| output.status)
        .map_err(|error| ProcessTreeError::Failed(error.to_string()))
}

fn process_exists(pid: i32) -> Result<bool, ProcessTreeError> {
    let output = gent_types::hidden_command("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .map_err(|error| ProcessTreeError::Failed(error.to_string()))?;
    if !output.status.success() {
        return Err(ProcessTreeError::Failed(format!(
            "tasklist exited with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.split("\",\"")
            .nth(1)
            .and_then(|value| value.parse::<i32>().ok())
            == Some(pid)
    }))
}

pub(super) fn bind_to_daemon(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    static DAEMON: std::sync::OnceLock<Option<win32job::Job>> = std::sync::OnceLock::new();
    let job = DAEMON.get_or_init(|| {
        let mut limits = win32job::ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close().limit_breakaway_ok();
        win32job::Job::create_with_limit_info(&limits).ok()
    });
    if let Some(job) = job {
        let _ = job.assign_process(child.as_raw_handle() as isize);
    }
}

pub(super) fn image_name(pid: u32) -> Option<String> {
    let output = gent_types::hidden_command("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    image_name_from(&String::from_utf8_lossy(&output.stdout), pid)
}

fn image_name_from(listing: &str, pid: u32) -> Option<String> {
    listing.lines().find_map(|line| {
        let mut fields = line.trim().trim_matches('"').split("\",\"");
        let image = fields.next()?;
        (fields.next()?.parse::<u32>().ok()? == pid).then(|| image.to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::ExitStatusExt;
    use std::process::Stdio;

    #[test]
    fn a_listed_process_is_identified_by_its_image_name() {
        let listing = "\"llama-server.exe\",\"4242\",\"Console\",\"1\",\"3,900,000 K\"
";
        assert_eq!(
            image_name_from(listing, 4242).as_deref(),
            Some("llama-server.exe")
        );
        assert_eq!(image_name_from(listing, 4243), None);
        assert_eq!(
            image_name_from(
                "INFO: No tasks are running which match the specified criteria.",
                4242
            ),
            None
        );
    }

    #[test]
    fn a_recorded_process_tree_is_reaped_when_its_image_still_owns_the_pid() {
        let directory = std::env::temp_dir().join(format!("gent-groups-{}", std::process::id()));
        let groups = crate::process::groups::ProcessGroups::open(&directory).unwrap();
        let mut child = gent_types::hidden_command("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        groups
            .record(child.id(), std::path::Path::new("PING.EXE"))
            .unwrap();
        assert_eq!(groups.stop_recorded().unwrap(), [child.id()]);
        assert!(child.wait().is_ok());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_hard_killed_owner_takes_its_provider_processes_with_it() {
        if std::env::var_os("GENT_DAEMON_JOB_OWNER").is_some() {
            let mut child = gent_types::hidden_command("ping")
                .args(["-n", "120", "127.0.0.1"])
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            crate::process::groups::spawned(&child, std::path::Path::new("PING.EXE"));
            println!("provider-pid={}", child.id());
            let _ = child.wait();
            return;
        }
        let mut owner = gent_types::hidden_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::windows::tests::a_hard_killed_owner_takes_its_provider_processes_with_it",
                "--nocapture",
            ])
            .env("GENT_DAEMON_JOB_OWNER", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = std::io::BufReader::new(owner.stdout.take().unwrap());
        let pid = std::io::BufRead::lines(stdout)
            .map_while(Result::ok)
            .find_map(|line| {
                line.split("provider-pid=")
                    .nth(1)
                    .and_then(|pid| pid.trim().parse::<u32>().ok())
            })
            .unwrap();
        assert_eq!(image_name(pid).as_deref(), Some("ping.exe"));
        owner.kill().unwrap();
        owner.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while image_name(pid).is_some() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(image_name(pid), None);
    }

    #[test]
    fn soft_refusal_escalates_only_the_owned_process_tree() {
        let mut calls = Vec::new();
        let result = signal_with(
            17,
            ProcessTreeSignal::Terminate,
            |pid, force| {
                calls.push((pid, force));
                Ok(ExitStatus::from_raw(u32::from(!force)))
            },
            |_| Ok(true),
        );
        assert!(result.is_ok());
        assert_eq!(calls, [(17, false), (17, true)]);
    }

    #[test]
    fn an_already_exited_tree_is_success_but_a_live_failure_is_reported() {
        for exists in [false, true] {
            let result = signal_with(
                17,
                ProcessTreeSignal::Kill,
                |_, _| Ok(ExitStatus::from_raw(128)),
                |_| Ok(exists),
            );
            assert_eq!(result.is_ok(), !exists);
        }
    }

    #[test]
    fn invalid_process_ids_never_reach_taskkill() {
        let result = signal_with(
            0,
            ProcessTreeSignal::Kill,
            |_, _| panic!("invalid process ID reached taskkill"),
            |_| Ok(false),
        );
        assert!(result.is_err());
    }

    #[test]
    fn real_windows_tree_stops_and_exited_process_is_detected() {
        let mut command = gent_types::hidden_command("cmd");
        let mut child = command
            .args(["/C", "ping", "-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start owned process tree");
        let pid = i32::try_from(child.id()).expect("process ID");
        assert!(process_exists(pid).expect("query owned process"));
        let result = signal_tree(pid, ProcessTreeSignal::Terminate);
        if result.is_err() {
            let _ = taskkill(pid, true);
        }
        child.wait().expect("wait for owned process");
        assert!(result.is_ok(), "{result:?}");
        assert!(!process_exists(pid).expect("query exited process"));
    }
}
