//! Bounded stdout chunk handling for one public-provider session.

use std::collections::VecDeque;

use crate::buffering::{BufferPolicy, FrameBuffer, OfferResult, ReadDirective};
use crate::ndjson::{NdjsonError, NdjsonFramer};

/// A fixed maximum for one operating-system stdout read before the pump rejects it.
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 4096;

/// Owns stdout framing and the backpressure boundary between process I/O and session reduction.
#[derive(Debug)]
pub struct ProviderOutputPump {
    framer: NdjsonFramer,
    buffer: FrameBuffer,
    pending: VecDeque<Vec<u8>>,
    max_chunk_bytes: usize,
}

impl ProviderOutputPump {
    /// Creates a pump that never accepts an unbounded OS read or raw provider frame.
    ///
    /// # Errors
    /// Returns an error for invalid NDJSON or frame-buffer limits.
    pub fn new(
        max_chunk_bytes: usize,
        max_frame_bytes: usize,
        policy: BufferPolicy,
    ) -> Result<Self, OutputPumpError> {
        if max_chunk_bytes == 0 {
            return Err(OutputPumpError::ZeroChunkLimit);
        }
        if max_frame_bytes > policy.max_bytes {
            return Err(OutputPumpError::FrameLimitExceedsBuffer {
                frame_limit: max_frame_bytes,
                buffer_limit: policy.max_bytes,
            });
        }
        Ok(Self {
            framer: NdjsonFramer::new(max_frame_bytes)?,
            buffer: FrameBuffer::new(policy),
            pending: VecDeque::new(),
            max_chunk_bytes,
        })
    }

    /// Frames one stdout chunk and stops future reads as soon as the buffer applies pressure.
    ///
    /// Callers must retain and retry a chunk when this returns [`OutputPumpError::Paused`].
    /// Complete frames already parsed from an accepted chunk remain FIFO in this pump.
    ///
    /// # Errors
    /// Returns an error before accepting an oversized chunk, or after recording any complete
    /// frames that precede a malformed provider line.
    pub fn accept_chunk(&mut self, chunk: &[u8]) -> Result<ReadDirective, OutputPumpError> {
        if !self.pending.is_empty() {
            return Err(OutputPumpError::Paused);
        }
        if chunk.len() > self.max_chunk_bytes {
            return Err(OutputPumpError::ChunkTooLarge {
                actual: chunk.len(),
                limit: self.max_chunk_bytes,
            });
        }
        for &byte in chunk {
            match self.framer.push_byte(byte) {
                Ok(Some(frame)) => self.pending.push_back(frame),
                Ok(None) => {}
                Err(error) => {
                    let _ = self.flush_pending()?;
                    return Err(error.into());
                }
            }
        }
        self.flush_pending()
    }

    /// Offers one already complete provider frame through the same bounded backpressure policy.
    ///
    /// This is retained for transports that already delimit provider frames before this edge.
    pub fn offer_frame(&mut self, frame: Vec<u8>) -> OfferResult {
        if !self.pending.is_empty() {
            return OfferResult::Backpressured;
        }
        self.buffer.offer(frame)
    }

    /// Removes one raw frame for the session reducer and resumes pending delivery when allowed.
    #[must_use]
    pub fn take_frame(&mut self) -> (Option<Vec<u8>>, Option<ReadDirective>) {
        let (frame, directive) = self.buffer.take();
        let directive = match directive {
            Some(ReadDirective::Resume) => match self.flush_pending() {
                Ok(ReadDirective::Pause) | Err(_) => Some(ReadDirective::Pause),
                Ok(ReadDirective::Continue | ReadDirective::Resume) => Some(ReadDirective::Resume),
            },
            other => other,
        };
        (frame, directive)
    }

    /// Returns all complete frames retained across the active buffer and pending suffix.
    #[must_use]
    pub fn queued_frames(&self) -> usize {
        self.buffer.queued_frames() + self.pending.len()
    }

    fn flush_pending(&mut self) -> Result<ReadDirective, OutputPumpError> {
        while let Some(frame) = self.pending.front() {
            match self.buffer.offer(frame.clone()) {
                OfferResult::Queued(directive) => {
                    self.pending.pop_front();
                    if directive == ReadDirective::Pause {
                        return Ok(ReadDirective::Pause);
                    }
                }
                OfferResult::Backpressured => return Ok(ReadDirective::Pause),
                OfferResult::RejectedOversize => {
                    let length = frame.len();
                    self.pending.pop_front();
                    return Err(OutputPumpError::RejectedFrame { length });
                }
            }
        }
        Ok(ReadDirective::Continue)
    }
}

/// A bounded stdout pump rejected input before it could enter the session buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OutputPumpError {
    #[error("provider output chunk limit must be non-zero")]
    ZeroChunkLimit,
    #[error("provider output chunk was {actual} bytes, above the {limit}-byte limit")]
    ChunkTooLarge { actual: usize, limit: usize },
    #[error("provider stdout reads are paused until queued frames drain")]
    Paused,
    #[error("provider frame limit {frame_limit} exceeds buffer limit {buffer_limit}")]
    FrameLimitExceedsBuffer {
        frame_limit: usize,
        buffer_limit: usize,
    },
    #[error("provider emitted a complete {length}-byte frame rejected by the session buffer")]
    RejectedFrame { length: usize },
    #[error(transparent)]
    Ndjson(#[from] NdjsonError),
}

#[cfg(test)]
mod tests {
    use super::{MAX_OUTPUT_CHUNK_BYTES, OutputPumpError, ProviderOutputPump};
    use crate::buffering::{BufferPolicy, ReadDirective};

    fn pump(max_frames: usize) -> ProviderOutputPump {
        ProviderOutputPump::new(
            MAX_OUTPUT_CHUNK_BYTES,
            64,
            BufferPolicy::new(max_frames, 128, 0, 0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn fragmented_stdout_frames_arrive_in_fifo_order() {
        let mut pump = pump(3);
        assert_eq!(pump.accept_chunk(b"{\"a\""), Ok(ReadDirective::Continue));
        assert_eq!(
            pump.accept_chunk(b":1}\r\n\n{\"b\":2}\n"),
            Ok(ReadDirective::Continue)
        );
        assert_eq!(pump.take_frame().0.unwrap(), br#"{"a":1}"#);
        assert_eq!(pump.take_frame().0.unwrap(), br#"{"b":2}"#);
    }

    #[test]
    fn pending_stdout_is_retained_until_hysteresis_resumes_reads() {
        let mut pump = pump(1);
        assert_eq!(pump.accept_chunk(b"one\ntwo\n"), Ok(ReadDirective::Pause));
        assert_eq!(pump.queued_frames(), 2);
        assert_eq!(pump.accept_chunk(b"three\n"), Err(OutputPumpError::Paused));
        let (first, directive) = pump.take_frame();
        assert_eq!(first.unwrap(), b"one");
        assert_eq!(directive, Some(ReadDirective::Pause));
        let (second, directive) = pump.take_frame();
        assert_eq!(second.unwrap(), b"two");
        assert_eq!(directive, Some(ReadDirective::Resume));
        assert_eq!(pump.accept_chunk(b"three\n"), Ok(ReadDirective::Pause));
        assert_eq!(pump.take_frame().0.unwrap(), b"three");
    }

    #[test]
    fn malformed_size_never_reuses_the_rejected_partial_line() {
        let mut pump = ProviderOutputPump::new(
            MAX_OUTPUT_CHUNK_BYTES,
            3,
            BufferPolicy::new(2, 8, 0, 0).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            pump.accept_chunk(b"abcd"),
            Err(OutputPumpError::Ndjson(_))
        ));
        assert_eq!(pump.accept_chunk(b"ok\n"), Ok(ReadDirective::Continue));
        assert_eq!(pump.take_frame().0.unwrap(), b"ok");
    }

    #[test]
    fn oversized_os_reads_are_rejected_before_framing() {
        let mut pump =
            ProviderOutputPump::new(2, 8, BufferPolicy::new(1, 8, 0, 0).unwrap()).unwrap();
        assert_eq!(
            pump.accept_chunk(b"abc"),
            Err(OutputPumpError::ChunkTooLarge {
                actual: 3,
                limit: 2,
            })
        );
        assert_eq!(pump.queued_frames(), 0);
    }

    #[test]
    fn frame_limit_cannot_exceed_the_bounded_buffer() {
        assert!(matches!(
            ProviderOutputPump::new(8, 9, BufferPolicy::new(1, 8, 0, 0).unwrap()),
            Err(OutputPumpError::FrameLimitExceedsBuffer {
                frame_limit: 9,
                buffer_limit: 8,
            })
        ));
    }
}
