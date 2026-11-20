use std::io::{Error, ErrorKind, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use super::{ProviderInput, QUEUED_FRAMES};
use crate::interrupt::ProcessTreeError;

struct Stalled(Receiver<()>);

impl Write for Stalled {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.recv();
        Err(Error::from(ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Recorded {
    written: Vec<u8>,
    fail: bool,
    done: Sender<Vec<u8>>,
}

impl Write for Recorded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail {
            return Err(Error::from(ErrorKind::BrokenPipe));
        }
        self.written.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Recorded {
    fn drop(&mut self) {
        let _ = self.done.send(std::mem::take(&mut self.written));
    }
}

fn recorded(fail: bool) -> (ProviderInput, Receiver<Vec<u8>>) {
    let (done, closed) = channel();
    let input = ProviderInput::spawn(Recorded {
        written: Vec::new(),
        fail,
        done,
    })
    .unwrap();
    (input, closed)
}

#[test]
fn a_provider_that_stops_reading_backlogs_without_blocking_the_sender() {
    let (release, stalled) = channel();
    let input = ProviderInput::spawn(Stalled(stalled)).unwrap();
    let outcome = (0..=QUEUED_FRAMES + 1)
        .map(|_| input.send(b"frame\n"))
        .find(Result::is_err);
    assert!(matches!(
        outcome,
        Some(Err(ProcessTreeError::InputBacklogged))
    ));
    drop(release);
}

#[test]
fn a_broken_input_closes_every_later_write() {
    let (input, closed) = recorded(true);
    input.send(b"first\n").unwrap();
    closed.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        input.send(b"second\n"),
        Err(ProcessTreeError::InputClosed)
    ));
}

#[test]
fn closing_writes_queued_frames_in_order_before_input_ends() {
    let (input, closed) = recorded(false);
    input.send(b"one\n").unwrap();
    input.send(b"two\n").unwrap();
    input.close();
    assert!(matches!(
        input.send(b"three\n"),
        Err(ProcessTreeError::InputClosed)
    ));
    assert_eq!(
        closed.recv_timeout(Duration::from_secs(10)).unwrap(),
        b"one\ntwo\n"
    );
}
