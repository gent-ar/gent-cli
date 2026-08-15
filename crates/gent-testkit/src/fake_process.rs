//! A deterministic process double for driver and runtime tests.
#![allow(clippy::missing_panics_doc)] // Test fakes fail fast on poisoned state.

use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeProcessSignal {
    Interrupt,
    Terminate,
    Kill,
}

#[derive(Debug, Default)]
struct ProcessState {
    stdin: Vec<Vec<u8>>,
    stdout: VecDeque<Vec<u8>>,
    stderr: VecDeque<Vec<u8>>,
    signals: Vec<FakeProcessSignal>,
    exit_code: Option<i32>,
}

/// An in-memory process script. Reads consume queued chunks in FIFO order.
#[derive(Debug, Default)]
pub struct FakeProcess {
    state: Mutex<ProcessState>,
}

impl FakeProcess {
    /// Queues a stdout chunk.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn push_stdout(&self, chunk: impl Into<Vec<u8>>) {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stdout
            .push_back(chunk.into());
    }

    /// Queues a stderr chunk.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn push_stderr(&self, chunk: impl Into<Vec<u8>>) {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stderr
            .push_back(chunk.into());
    }

    /// Records one stdin write.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn write_stdin(&self, chunk: impl Into<Vec<u8>>) {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stdin
            .push(chunk.into());
    }

    /// Removes the next queued stdout chunk.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn read_stdout(&self) -> Option<Vec<u8>> {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stdout
            .pop_front()
    }

    /// Removes the next queued stderr chunk.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn read_stderr(&self) -> Option<Vec<u8>> {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stderr
            .pop_front()
    }

    /// Records one process-tree signal.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn signal(&self, signal: FakeProcessSignal) {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .signals
            .push(signal);
    }

    /// Records the process exit code.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn exit(&self, code: i32) {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .exit_code = Some(code);
    }

    /// Returns stdin writes in their original order.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn stdin(&self) -> Vec<Vec<u8>> {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .stdin
            .clone()
    }

    /// Returns process-tree signals in their original order.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn signals(&self) -> Vec<FakeProcessSignal> {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .signals
            .clone()
    }

    /// Returns the recorded process exit code, if it has exited.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.state
            .lock()
            .expect("fake process mutex poisoned")
            .exit_code
    }
}
