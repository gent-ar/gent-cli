//! Bounded newline-delimited JSON framing for provider standard-output chunks.

/// Reassembles complete NDJSON frames without retaining an unbounded partial line.
#[derive(Debug)]
pub struct NdjsonFramer {
    max_frame_bytes: usize,
    partial: Vec<u8>,
    discarding: bool,
    skipped: usize,
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
            discarding: false,
            skipped: 0,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        chunk
            .iter()
            .filter_map(|&byte| self.push_byte(byte))
            .collect()
    }

    pub fn push_byte(&mut self, byte: u8) -> Option<Vec<u8>> {
        if byte == b'\n' {
            if std::mem::take(&mut self.discarding) {
                return None;
            }
            if self.partial.last() == Some(&b'\r') {
                self.partial.pop();
            }
            return (!self.partial.is_empty()).then(|| std::mem::take(&mut self.partial));
        }
        if self.discarding {
            return None;
        }
        if self.partial.len() == self.max_frame_bytes {
            self.partial = Vec::new();
            self.discarding = true;
            self.skipped += 1;
            return None;
        }
        self.partial.push(byte);
        None
    }

    pub fn take_skipped_frames(&mut self) -> usize {
        std::mem::take(&mut self.skipped)
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
}

#[cfg(test)]
mod tests {
    use super::{NdjsonError, NdjsonFramer};

    #[test]
    fn frames_fragmented_and_multiple_lines_in_order() {
        let mut framer = NdjsonFramer::new(16).unwrap();
        assert!(framer.push(b"{\"a\"").is_empty());
        assert_eq!(
            framer.push(b":1}\r\n\n{\"b\":2}\n"),
            [b"{\"a\":1}", b"{\"b\":2}"]
        );
        assert_eq!(framer.partial_len(), 0);
    }

    #[test]
    fn an_oversized_line_is_skipped_through_its_newline_without_stopping_the_stream() {
        let mut framer = NdjsonFramer::new(3).unwrap();
        assert!(framer.push(b"abcd").is_empty());
        assert_eq!(framer.partial_len(), 0);
        assert!(framer.push(b"still the same line\n").is_empty());
        assert_eq!(framer.push(b"ok\n"), [b"ok"]);
        assert_eq!(framer.take_skipped_frames(), 1);
        assert_eq!(framer.take_skipped_frames(), 0);
    }

    #[test]
    fn zero_limit_is_rejected() {
        assert!(matches!(NdjsonFramer::new(0), Err(NdjsonError::ZeroLimit)));
    }
}
