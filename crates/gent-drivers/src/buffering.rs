//! Bounded raw-frame buffering policy. Transport adapters retain frames when reads pause.

use std::collections::VecDeque;

/// Fixed capacity and hysteresis limits for one provider stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferPolicy {
    pub max_frames: usize,
    pub max_bytes: usize,
    pub resume_frames: usize,
    pub resume_bytes: usize,
}

impl BufferPolicy {
    /// Constructs a policy whose resume watermarks cannot exceed capacity.
    ///
    /// # Errors
    /// Returns an error for zero capacity or watermarks above capacity.
    pub fn new(
        max_frames: usize,
        max_bytes: usize,
        resume_frames: usize,
        resume_bytes: usize,
    ) -> Result<Self, BufferPolicyError> {
        if max_frames == 0 || max_bytes == 0 {
            return Err(BufferPolicyError::ZeroCapacity);
        }
        if resume_frames > max_frames || resume_bytes > max_bytes {
            return Err(BufferPolicyError::InvalidResumeWatermark);
        }
        Ok(Self {
            max_frames,
            max_bytes,
            resume_frames,
            resume_bytes,
        })
    }
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum BufferPolicyError {
    #[error("buffer capacity must be non-zero")]
    ZeroCapacity,
    #[error("resume watermark cannot exceed capacity")]
    InvalidResumeWatermark,
}

/// Instruction for the I/O edge. It must stop reading before memory can grow unbounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadDirective {
    Continue,
    Pause,
    Resume,
}

/// Result of offering one complete raw frame to a bounded buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfferResult {
    Queued(ReadDirective),
    Backpressured,
    RejectedOversize,
}

/// In-memory state owned by a stream adapter; it never discards accepted frames.
#[derive(Debug)]
pub struct FrameBuffer {
    policy: BufferPolicy,
    frames: VecDeque<Vec<u8>>,
    queued_bytes: usize,
    paused: bool,
}

impl FrameBuffer {
    #[must_use]
    pub fn new(policy: BufferPolicy) -> Self {
        Self {
            policy,
            frames: VecDeque::new(),
            queued_bytes: 0,
            paused: false,
        }
    }

    /// Offers a frame. On backpressure, the caller keeps the unaccepted frame and stops reads.
    pub fn offer(&mut self, frame: Vec<u8>) -> OfferResult {
        if frame.len() > self.policy.max_bytes {
            return OfferResult::RejectedOversize;
        }
        if self.paused || !self.can_fit(frame.len()) {
            self.paused = true;
            return OfferResult::Backpressured;
        }
        self.queued_bytes += frame.len();
        self.frames.push_back(frame);
        if self.at_capacity() {
            self.paused = true;
            OfferResult::Queued(ReadDirective::Pause)
        } else {
            OfferResult::Queued(ReadDirective::Continue)
        }
    }

    /// Removes one frame and emits a single resume directive after crossing both watermarks.
    pub fn take(&mut self) -> (Option<Vec<u8>>, Option<ReadDirective>) {
        let frame = self.frames.pop_front();
        if let Some(frame) = &frame {
            self.queued_bytes -= frame.len();
        }
        let directive = (self.paused && self.can_resume()).then(|| {
            self.paused = false;
            ReadDirective::Resume
        });
        (frame, directive)
    }

    #[must_use]
    pub fn queued_frames(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub const fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }

    fn can_fit(&self, bytes: usize) -> bool {
        self.frames.len() < self.policy.max_frames
            && self.queued_bytes.saturating_add(bytes) <= self.policy.max_bytes
    }

    fn at_capacity(&self) -> bool {
        self.frames.len() == self.policy.max_frames || self.queued_bytes == self.policy.max_bytes
    }

    fn can_resume(&self) -> bool {
        self.frames.len() <= self.policy.resume_frames
            && self.queued_bytes <= self.policy.resume_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferPolicy, FrameBuffer, OfferResult, ReadDirective};

    fn policy() -> BufferPolicy {
        BufferPolicy::new(2, 8, 0, 0).unwrap()
    }

    #[test]
    fn full_buffer_pauses_without_dropping_the_accepted_frame() {
        let mut buffer = FrameBuffer::new(policy());
        assert_eq!(
            buffer.offer(vec![1; 4]),
            OfferResult::Queued(ReadDirective::Continue)
        );
        assert_eq!(
            buffer.offer(vec![2; 4]),
            OfferResult::Queued(ReadDirective::Pause)
        );
        assert_eq!(buffer.offer(vec![3]), OfferResult::Backpressured);
        assert_eq!(buffer.queued_frames(), 2);
        assert_eq!(buffer.queued_bytes(), 8);
    }

    #[test]
    fn buffer_resumes_only_after_both_watermarks_are_crossed() {
        let mut buffer = FrameBuffer::new(policy());
        buffer.offer(vec![1; 4]);
        buffer.offer(vec![2; 4]);
        assert_eq!(buffer.take().1, None);
        assert_eq!(buffer.take().1, Some(ReadDirective::Resume));
    }

    #[test]
    fn oversize_frame_is_rejected_before_allocation_in_the_queue() {
        let mut buffer = FrameBuffer::new(policy());
        assert_eq!(buffer.offer(vec![0; 9]), OfferResult::RejectedOversize);
        assert_eq!(buffer.queued_frames(), 0);
    }
}
