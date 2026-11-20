use std::io::Write;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Mutex, PoisonError};
use std::thread;

use crate::interrupt::ProcessTreeError;

const QUEUED_FRAMES: usize = 64;

#[derive(Debug)]
pub struct ProviderInput {
    frames: Mutex<Option<SyncSender<Vec<u8>>>>,
}

impl ProviderInput {
    pub fn spawn(mut input: impl Write + Send + 'static) -> std::io::Result<Self> {
        let (sender, frames) = sync_channel::<Vec<u8>>(QUEUED_FRAMES);
        thread::Builder::new()
            .name("gent-provider-input".into())
            .spawn(move || {
                for frame in frames {
                    if input
                        .write_all(&frame)
                        .and_then(|()| input.flush())
                        .is_err()
                    {
                        return;
                    }
                }
            })?;
        Ok(Self {
            frames: Mutex::new(Some(sender)),
        })
    }

    pub fn send(&self, frame: &[u8]) -> Result<(), ProcessTreeError> {
        let frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        let sender = frames.as_ref().ok_or(ProcessTreeError::InputClosed)?;
        sender
            .try_send(frame.to_vec())
            .map_err(|error| match error {
                TrySendError::Full(_) => ProcessTreeError::InputBacklogged,
                TrySendError::Disconnected(_) => ProcessTreeError::InputClosed,
            })
    }

    pub fn close(&self) {
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

#[cfg(test)]
#[path = "provider_input_tests.rs"]
mod tests;
