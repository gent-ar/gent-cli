use std::collections::BTreeSet;
use std::io::Write;
use std::process::Child;
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::SandboxedProviderLaunchError as Error;
use gent_types::SandboxedLaunchRequest;

pub(crate) struct BootstrapFrame(Vec<u8>);

impl std::ops::Deref for BootstrapFrame {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}
impl std::ops::DerefMut for BootstrapFrame {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}
impl Drop for BootstrapFrame {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

pub(crate) fn encode(request: &SandboxedLaunchRequest) -> Result<BootstrapFrame, Error> {
    let mut frame = BootstrapFrame(Vec::new());
    if let Some(gateway) = request.profile.gateway() {
        let token = gateway.scoped_token().as_bytes();
        if token.len() != 64
            || !token
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(Error::ProfileRejected);
        }
        frame.extend_from_slice(b"GENTGW01");
        frame.extend_from_slice(token);
    }
    if request
        .profile
        .shell_runtime()
        .and_then(|shell| shell.powershell_lock())
        .is_some()
    {
        let paths: BTreeSet<_> = request
            .profile
            .readable_roots()
            .iter()
            .map(|p| p.as_path())
            .chain(
                request
                    .profile
                    .resources()
                    .iter()
                    .filter(|r| !r.writable())
                    .map(|r| r.root()),
            )
            .collect();
        if paths.len() > 1024 {
            return Err(Error::ProfileRejected);
        }
        let start = frame.len();
        frame.extend_from_slice(b"GENTRO01");
        frame.extend_from_slice(
            &u32::try_from(paths.len())
                .map_err(|_| Error::ProfileRejected)?
                .to_le_bytes(),
        );
        for path in paths {
            let text = path.to_str().ok_or(Error::ProfileRejected)?.as_bytes();
            if text.is_empty() || text.len() > 32768 || text.contains(&0) {
                return Err(Error::ProfileRejected);
            }
            frame.extend_from_slice(
                &u32::try_from(text.len())
                    .map_err(|_| Error::ProfileRejected)?
                    .to_le_bytes(),
            );
            frame.extend_from_slice(text);
            if frame.len() - start > 512 * 1024 {
                return Err(Error::ProfileRejected);
            }
        }
    }
    Ok(frame)
}

fn retain_writer(writer: JoinHandle<()>) {
    static WRITERS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    let mut owned = WRITERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    owned.retain(|thread| !thread.is_finished());
    owned.push(writer);
}

pub(crate) fn write_gateway_bootstrap(
    child: &mut Child,
    request: &SandboxedLaunchRequest,
) -> Result<(), Error> {
    write_with_deadline(child, encode(request)?, Duration::from_secs(10))
}

fn write_with_deadline(
    child: &mut Child,
    mut frame: BootstrapFrame,
    timeout: Duration,
) -> Result<(), Error> {
    if frame.is_empty() {
        return Ok(());
    }
    let mut stdin = child.stdin.take().ok_or(Error::ContainmentRejected)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let writer = thread::spawn(move || {
        let result = stdin.write_all(&frame);
        frame.fill(0);
        let _ = sender.send((stdin, result));
    });
    if let Ok((stdin, result)) = receiver.recv_timeout(timeout) {
        child.stdin = Some(stdin);
        writer.join().map_err(|_| Error::ContainmentRejected)?;
        return result.map_err(|_| Error::ContainmentRejected);
    }
    let _ = child.kill();
    if let Ok((stdin, _)) = receiver.recv_timeout(Duration::from_secs(2)) {
        drop(stdin);
        let _ = writer.join();
    } else {
        retain_writer(writer);
    }
    Err(Error::LaunchFailed("native sandbox bootstrap write did not settle before its deadline; scope remains unprovable".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn bootstrap_stall_child() {
        if std::env::var_os("GENT_BOOTSTRAP_STALL_CHILD").is_some() {
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn exact_owned_pipe_stall_is_bounded_and_never_returns_success() {
        let executable = std::env::current_exe().unwrap();
        let test = "windows_sandbox::config::bootstrap::tests::bootstrap_stall_child";
        let mut child = Command::new(executable)
            .args(["--exact", test, "--nocapture"])
            .env("GENT_BOOTSTRAP_STALL_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        let result = write_with_deadline(
            &mut child,
            BootstrapFrame(vec![b'Q'; 512 * 1024]),
            Duration::from_millis(100),
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(!child.wait().unwrap().success());
    }
}
