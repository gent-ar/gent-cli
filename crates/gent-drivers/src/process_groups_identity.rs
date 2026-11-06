#[cfg(target_os = "linux")]
use std::fs;
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(windows)]
pub(super) use super::super::windows::image_name as identity;

pub(super) fn still_owned(pid: u32, recorded: &str) -> bool {
    match identity(pid) {
        Some(current) => !recorded.is_empty() && current == recorded,
        None => group_alive(pid),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    fields.get(19).map(|started| (*started).to_owned())
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn identity(pid: u32) -> Option<String> {
    let output = Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let started = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !started.is_empty()).then_some(started)
}

#[cfg(not(any(unix, windows)))]
pub(super) const fn identity(_: u32) -> Option<String> {
    None
}

#[cfg(unix)]
fn group_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
const fn group_alive(_: u32) -> bool {
    false
}
