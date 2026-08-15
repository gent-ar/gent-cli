//! Bounded newline-delimited JSON framing for provider standard-output chunks.

/// Reassembles complete NDJSON frames without retaining an unbounded partial line.
#[derive(Debug)]
pub struct NdjsonFramer {
    max_frame_bytes: usize,
    partial: Vec<u8>,
}

impl NdjsonFramer {
    /// Creates a framer with a non-zero maximum complete-frame size.
    ///
    /// # Errors
    /// Returns an error when the configured frame limit is zero.
    pub const fn new(max_frame_bytes: usize) -> Result<Self, NdjsonError> {
        if max_frame_bytes == 0 {
            return Err(NdjsonError::ZeroLimit);
        }
        Ok(Self {
            max_frame_bytes,
            partial: Vec::new(),
        })
    }

    /// Adds one output chunk and returns every complete non-empty frame in order.
    ///
    /// # Errors
    /// Returns an error and discards the partial line when it exceeds the configured limit.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, NdjsonError> {
        let mut frames = Vec::new();
        for &byte in chunk {
            if let Some(frame) = self.push_byte(byte)? {
                frames.push(frame);
            }
        }
        Ok(frames)
    }

    /// Adds one byte and returns a completed non-empty frame, if one ended at this byte.
    ///
    /// # Errors
    /// Returns an error and discards the partial line when it exceeds the configured limit.
    pub fn push_byte(&mut self, byte: u8) -> Result<Option<Vec<u8>>, NdjsonError> {
        if byte == b'\n' {
            if self.partial.last() == Some(&b'\r') {
                self.partial.pop();
            }
            return Ok((!self.partial.is_empty()).then(|| std::mem::take(&mut self.partial)));
        }
        if self.partial.len() == self.max_frame_bytes {
            self.partial.clear();
            return Err(NdjsonError::FrameTooLarge);
        }
        self.partial.push(byte);
        Ok(None)
    }

    /// Returns the currently retained partial-frame length for observability and tests.
    #[must_use]
    pub fn partial_len(&self) -> usize {
        self.partial.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NdjsonError {
    #[error("NDJSON frame limit must be non-zero")]
    ZeroLimit,
    #[error("provider emitted an NDJSON frame larger than the configured limit")]
    FrameTooLarge,
}

#[cfg(test)]
mod tests {
    use super::{NdjsonError, NdjsonFramer};

    #[test]
    fn frames_fragmented_and_multiple_lines_in_order() {
        let mut framer = NdjsonFramer::new(16).unwrap();
        assert!(framer.push(b"{\"a\"").unwrap().is_empty());
        assert_eq!(
            framer.push(b":1}\r\n\n{\"b\":2}\n").unwrap(),
            [b"{\"a\":1}", b"{\"b\":2}"]
        );
        assert_eq!(framer.partial_len(), 0);
    }

    #[test]
    fn partial_line_is_bounded_and_reset_after_rejection() {
        let mut framer = NdjsonFramer::new(3).unwrap();
        assert_eq!(framer.push(b"abcd"), Err(NdjsonError::FrameTooLarge));
        assert_eq!(framer.partial_len(), 0);
        assert_eq!(framer.push(b"ok\n").unwrap(), [b"ok"]);
    }

    #[test]
    fn zero_limit_is_rejected() {
        assert!(matches!(NdjsonFramer::new(0), Err(NdjsonError::ZeroLimit)));
    }
}
