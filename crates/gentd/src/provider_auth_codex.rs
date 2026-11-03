use gent_drivers::{
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    supervisor::ProviderProcess,
};
use gent_types::{
    ProviderAuthHandoff, ProviderAuthLifecycle, ProviderAuthProvider, RunVersionLock,
};
use serde_json::Value;
use std::time::{Duration, Instant};

use super::provider_auth_process::{AuthProcess, launch_interactive};
#[path = "provider_auth_codex_state.rs"]
mod state;
use state::CodexAuthState;

pub(super) struct CodexDeviceLogin {
    process: AuthProcess,
    state: CodexAuthState,
    buffer: Vec<u8>,
    deadline: Instant,
    offer_deadline: Instant,
    expiry: u64,
}

impl CodexDeviceLogin {
    pub(super) fn start(
        lock: &RunVersionLock,
        expiry: u64,
        control: &crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ) -> Result<Self, String> {
        let process = launch_interactive(
            ProviderAuthProvider::Codex,
            lock,
            &["app-server", "-c", "check_for_update_on_startup=false"],
            control,
        )?;
        let login = Self {
            process,
            state: CodexAuthState::new(),
            buffer: Vec::new(),
            deadline: Instant::now()
                + Duration::from_secs(expiry.saturating_sub(super::provider_auth_process::now())),
            offer_deadline: Instant::now() + Duration::from_secs(30),
            expiry,
        };
        login.write(&CodexAuthState::initialize())?;
        Ok(login)
    }

    pub(super) fn handoff(&self) -> Option<ProviderAuthHandoff> {
        self.state.handoff.clone()
    }

    pub(super) fn refresh(&mut self) -> Option<ProviderAuthLifecycle> {
        if Instant::now() >= self.deadline {
            return Some(ProviderAuthLifecycle::TimedOut);
        }
        let polled = self.read();
        if polled.is_err() {
            return Some(ProviderAuthLifecycle::Failed);
        }
        if let Some(success) = self.state.completed {
            return Some(if success {
                ProviderAuthLifecycle::Authenticated
            } else {
                ProviderAuthLifecycle::Failed
            });
        }
        if self.state.handoff.is_none() && Instant::now() >= self.offer_deadline {
            return Some(ProviderAuthLifecycle::TimedOut);
        }
        match self.process.try_exit_code() {
            Ok(None) => None,
            _ => Some(ProviderAuthLifecycle::Failed),
        }
    }

    fn read(&mut self) -> Result<(), ()> {
        let mut chunks = 0;
        while let Some(chunk) = self.process.next_stdout_chunk().map_err(|_| ())? {
            self.buffer.extend_from_slice(&chunk);
            while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                if end > 64 * 1024 {
                    return Err(());
                }
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let value: Value = serde_json::from_slice(&line).map_err(|_| ())?;
                for frame in self.state.receive(&value, self.expiry)? {
                    self.write(&frame).map_err(|_| ())?;
                }
            }
            if self.buffer.len() > 64 * 1024 {
                return Err(());
            }
            chunks += 1;
            if chunks == 128 {
                break;
            }
        }
        Ok(())
    }

    fn write(&self, value: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|_| "provider authentication request failed".to_owned())?;
        bytes.push(b'\n');
        self.process
            .write_frame(&bytes)
            .map_err(|_| "provider authentication request failed".to_owned())
    }
}

impl Drop for CodexDeviceLogin {
    fn drop(&mut self) {
        if let Some(cancel) = self.state.cancel() {
            let _ = self.write(&cancel);
        }
        let _ = self.process.close_stdin();
        let _ = self.process.signal_tree(ProcessTreeSignal::Kill);
    }
}
