use std::os::windows::process::CommandExt;
use std::process::{Command, ExitStatus};

use crate::interrupt::{ProcessTreeError, ProcessTreeSignal};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    let mut command = Command::new("taskkill");
    command.creation_flags(CREATE_NO_WINDOW);
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
    let output = Command::new("tasklist")
        .creation_flags(CREATE_NO_WINDOW)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::ExitStatusExt;
    use std::process::Stdio;

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
        let mut command = Command::new("cmd");
        command.creation_flags(CREATE_NO_WINDOW);
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
