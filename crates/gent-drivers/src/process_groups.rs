#[cfg(unix)]
use std::process::{Command, Stdio};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{OnceLock, PoisonError},
};

use super::{SystemProcess, signal_process_tree};
use crate::interrupt::{ProcessTreeError, ProcessTreeSignal};

static ACTIVE: OnceLock<ProcessGroups> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct ProcessGroups {
    directory: PathBuf,
}

#[derive(Debug)]
pub struct StopRecordedOnExit(ProcessGroups);

impl ProcessGroups {
    pub fn open(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    pub fn record(&self, pid: u32) -> io::Result<()> {
        fs::write(self.record_path(pid), identity(pid).unwrap_or_default())
    }

    pub fn forget(&self, pid: u32) {
        let _ = fs::remove_file(self.record_path(pid));
    }

    pub fn stop_recorded(&self) -> io::Result<Vec<u32>> {
        let mut stopped = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            let recorded = fs::read_to_string(&path).unwrap_or_default();
            let pid = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse::<u32>().ok());
            if let Some(pid) = pid
                && still_owned(pid, &recorded)
                && i32::try_from(pid)
                    .is_ok_and(|group| signal_process_tree(group, ProcessTreeSignal::Kill).is_ok())
            {
                stopped.push(pid);
            }
            let _ = fs::remove_file(&path);
        }
        stopped.sort_unstable();
        Ok(stopped)
    }

    fn record_path(&self, pid: u32) -> PathBuf {
        self.directory.join(pid.to_string())
    }
}

impl StopRecordedOnExit {
    #[must_use]
    pub const fn new(groups: ProcessGroups) -> Self {
        Self(groups)
    }
}

impl Drop for StopRecordedOnExit {
    fn drop(&mut self) {
        let _ = self.0.stop_recorded();
    }
}

pub fn install(directory: &Path) -> io::Result<(StopRecordedOnExit, Vec<u32>)> {
    let groups = ProcessGroups::open(directory)?;
    let survivors = groups.stop_recorded()?;
    let _ = ACTIVE.set(groups.clone());
    Ok((StopRecordedOnExit::new(groups), survivors))
}

pub fn spawned(pid: u32) {
    if let Some(groups) = ACTIVE.get() {
        let _ = groups.record(pid);
    }
}

pub fn reaped(pid: u32) {
    if let Some(groups) = ACTIVE.get() {
        groups.forget(pid);
    }
}

impl Drop for SystemProcess {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        let pid = child.id();
        if matches!(child.try_wait(), Ok(None)) {
            if let Ok(group) = i32::try_from(pid) {
                let _ = signal_process_tree(group, ProcessTreeSignal::Kill);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        reaped(pid);
    }
}

fn still_owned(pid: u32, recorded: &str) -> bool {
    match identity(pid) {
        Some(current) => !recorded.is_empty() && current == recorded,
        None => group_alive(pid),
    }
}

#[cfg(target_os = "linux")]
fn identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    fields.get(19).map(|started| (*started).to_owned())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn identity(pid: u32) -> Option<String> {
    let output = Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let started = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !started.is_empty()).then_some(started)
}

#[cfg(not(unix))]
const fn identity(_: u32) -> Option<String> {
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

#[cfg(unix)]
pub(super) fn signal_tree(group: i32, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
    let name = match signal {
        ProcessTreeSignal::Interrupt => "INT",
        ProcessTreeSignal::Terminate => "TERM",
        ProcessTreeSignal::Kill => "KILL",
    };
    let escaped = if signal == ProcessTreeSignal::Kill {
        let _ = kill("STOP", &[-group]);
        frozen_escaped_descendants(group)
    } else {
        escaped_descendants(group)
    };
    let delivered = kill(name, &[-group]);
    if !escaped.is_empty() {
        let _ = kill(name, &escaped);
    }
    delivered
}

#[cfg(unix)]
fn frozen_escaped_descendants(group: i32) -> Vec<i32> {
    let mut frozen = BTreeSet::new();
    loop {
        let found = escaped_descendants(group)
            .into_iter()
            .filter(|pid| !frozen.contains(pid))
            .collect::<Vec<_>>();
        if found.is_empty() {
            return frozen.into_iter().collect();
        }
        let _ = kill("STOP", &found);
        frozen.extend(found);
    }
}

#[cfg(unix)]
fn kill(signal: &str, targets: &[i32]) -> Result<(), ProcessTreeError> {
    let status = Command::new("/bin/kill")
        .args(["-s", signal, "--"])
        .args(targets.iter().map(ToString::to_string))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| ProcessTreeError::Failed(error.to_string()))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| ProcessTreeError::Failed(format!("kill exited with {status}")))
}

#[cfg(unix)]
fn escaped_descendants(group: i32) -> Vec<i32> {
    let table = process_table();
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    let mut tree = table
        .iter()
        .filter(|entry| entry.pgid == group)
        .map(|entry| entry.pid)
        .collect::<BTreeSet<_>>();
    loop {
        let children = table
            .iter()
            .filter(|entry| tree.contains(&entry.ppid) && !tree.contains(&entry.pid))
            .map(|entry| entry.pid)
            .collect::<Vec<_>>();
        if children.is_empty() {
            break;
        }
        tree.extend(children);
    }
    table
        .iter()
        .filter(|entry| tree.contains(&entry.pid) && entry.pgid != group)
        .map(|entry| entry.pid)
        .filter(|pid| *pid > 1 && *pid != own)
        .collect()
}

#[cfg(unix)]
struct ProcessEntry {
    pid: i32,
    ppid: i32,
    pgid: i32,
}

#[cfg(target_os = "linux")]
fn process_table() -> Vec<ProcessEntry> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let pid = entry.ok()?.file_name().to_str()?.parse::<i32>().ok()?;
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let fields = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            Some(ProcessEntry {
                pid,
                ppid: fields.get(1)?.parse().ok()?,
                pgid: fields.get(2)?.parse().ok()?,
            })
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_table() -> Vec<ProcessEntry> {
    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<i32>);
            Some(ProcessEntry {
                pid: fields.next()?.ok()?,
                ppid: fields.next()?.ok()?,
                pgid: fields.next()?.ok()?,
            })
        })
        .collect()
}
