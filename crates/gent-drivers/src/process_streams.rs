//! Bounded process-pipe readers that preserve stdout for the provider reducer.

use std::io::Read;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

const STDOUT_QUEUE_CHUNKS: usize = 16;

/// Bounded diagnostic capture from one provider stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedStream {
    pub bytes: Vec<u8>,
    pub discarded_bytes: usize,
}

/// Bounded diagnostic captures from a public provider process.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessOutput {
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
}

/// Owns the bounded readers for one provider process.
#[derive(Debug)]
pub(crate) struct ProcessStreams {
    output: Arc<Mutex<ProcessOutput>>,
    stdout: Mutex<Receiver<Vec<u8>>>,
    readers: Mutex<Vec<JoinHandle<()>>>,
}

impl ProcessStreams {
    pub(crate) fn new(
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        output_limit: usize,
    ) -> Self {
        let output = Arc::new(Mutex::new(ProcessOutput::default()));
        let (sender, receiver) = sync_channel(STDOUT_QUEUE_CHUNKS);
        let readers = vec![
            start_reader(
                stdout,
                &output,
                output_limit,
                StreamKind::Stdout,
                Some(sender),
            ),
            start_reader(stderr, &output, output_limit, StreamKind::Stderr, None),
        ];
        Self {
            output,
            stdout: Mutex::new(receiver),
            readers: Mutex::new(readers),
        }
    }

    #[must_use]
    pub(crate) fn output(&self) -> ProcessOutput {
        recover_lock(&self.output).clone()
    }

    #[must_use]
    pub(crate) fn next_stdout_chunk(&self) -> Option<Vec<u8>> {
        recover_lock(&self.stdout).try_recv().ok()
    }

    pub(crate) fn join_after_exit(&self) {
        let mut readers = recover_lock(&self.readers).drain(..).collect::<Vec<_>>();
        while readers.iter().any(|reader| !reader.is_finished()) {
            let _ = self.next_stdout_chunk();
            thread::yield_now();
        }
        for reader in readers.drain(..) {
            let _ = reader.join();
        }
    }
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

fn start_reader(
    reader: impl Read + Send + 'static,
    output: &Arc<Mutex<ProcessOutput>>,
    limit: usize,
    stream: StreamKind,
    sender: Option<SyncSender<Vec<u8>>>,
) -> JoinHandle<()> {
    let output = Arc::clone(output);
    thread::spawn(move || read_bounded(reader, &output, limit, stream, sender.as_ref()))
}

fn read_bounded(
    mut reader: impl Read,
    output: &Arc<Mutex<ProcessOutput>>,
    limit: usize,
    stream: StreamKind,
    sender: Option<&SyncSender<Vec<u8>>>,
) {
    let mut buffer = [0_u8; crate::output_pump::MAX_OUTPUT_CHUNK_BYTES];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            return;
        }
        let chunk = &buffer[..read];
        append(&mut recover_lock(output), chunk, limit, stream);
        if let Some(sender) = &sender
            && sender.send(chunk.to_vec()).is_err()
        {
            return;
        }
    }
}

fn append(output: &mut ProcessOutput, chunk: &[u8], limit: usize, stream: StreamKind) {
    let target = match stream {
        StreamKind::Stdout => &mut output.stdout,
        StreamKind::Stderr => &mut output.stderr,
    };
    let accepted = limit.saturating_sub(target.bytes.len()).min(chunk.len());
    target.bytes.extend_from_slice(&chunk[..accepted]);
    target.discarded_bytes += chunk.len() - accepted;
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
