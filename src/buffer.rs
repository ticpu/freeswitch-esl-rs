//! Buffer management for ESL protocol parsing

use crate::{
    constants::*,
    error::{EslError, EslResult},
};
use bytes::{Buf, BufMut, BytesMut};

/// Buffer wrapper for efficient ESL protocol parsing
pub struct EslBuffer {
    buffer: BytesMut,
    /// How far into `buffer` we have already scanned for a pattern.
    /// Backed up by `pattern_len - 1` on each new search to catch patterns
    /// that straddle the previously-scanned boundary.
    scan_offset: usize,
}

impl EslBuffer {
    /// Create new buffer with default capacity
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(BUF_CHUNK),
            scan_offset: 0,
        }
    }

    /// Get current length of unconsumed data in buffer
    pub fn len(&self) -> usize {
        self.buffer
            .len()
    }

    /// Extend buffer with more data
    pub fn extend_from_slice(&mut self, data: &[u8]) {
        if self
            .buffer
            .remaining_mut()
            < data.len()
        {
            let old_cap = self
                .buffer
                .capacity();
            let new_space = data
                .len()
                .max(BUF_CHUNK);
            self.buffer
                .reserve(new_space);
            tracing::debug!(
                "Buffer grew from {} to {} bytes (added {} bytes)",
                old_cap,
                self.buffer
                    .capacity(),
                self.buffer
                    .capacity()
                    - old_cap
            );
        }
        self.buffer
            .extend_from_slice(data);
    }

    /// Get reference to unconsumed data
    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    /// Consume bytes from the front of the buffer.
    ///
    /// Returns `Err` if `count` exceeds the available data.
    pub fn advance(&mut self, count: usize) -> EslResult<()> {
        let available = self.len();
        if count > available {
            return Err(EslError::protocol_error(format!(
                "cannot advance {} bytes, only {} available",
                count, available
            )));
        }
        self.buffer
            .advance(count);
        self.scan_offset = self
            .scan_offset
            .saturating_sub(count);
        Ok(())
    }

    /// Find the position of `pattern` in the buffer using `memchr::memmem`.
    ///
    /// A scan-offset watermark avoids re-scanning bytes that were already
    /// searched in a previous call. On each call the search starts from
    /// `watermark - (pattern.len() - 1)` to catch patterns that straddle
    /// the old boundary. The watermark advances to the end of the buffer on
    /// a miss, and to just past the found pattern on a hit.
    pub fn find_pattern(&mut self, pattern: &[u8]) -> Option<usize> {
        if pattern.is_empty()
            || self
                .buffer
                .len()
                < pattern.len()
        {
            return None;
        }
        // Back up enough to catch a pattern straddling the previous watermark
        let start = self
            .scan_offset
            .saturating_sub(
                pattern
                    .len()
                    .saturating_sub(1),
            );
        let search_slice = &self.buffer[start..];
        if let Some(rel_pos) = memchr::memmem::find(search_slice, pattern) {
            let abs_pos = start + rel_pos;
            self.scan_offset = abs_pos + pattern.len();
            Some(abs_pos)
        } else {
            self.scan_offset = self
                .buffer
                .len();
            None
        }
    }

    /// Extract data up to (but not including) the pattern, consuming through
    /// the end of the pattern.
    pub fn extract_until_pattern(&mut self, pattern: &[u8]) -> Option<Vec<u8>> {
        let pos = self.find_pattern(pattern)?;
        let result = self.buffer[..pos].to_vec();
        self.buffer
            .advance(pos + pattern.len());
        self.scan_offset = 0;
        Some(result)
    }

    /// Extract exact number of bytes from the front of the buffer.
    pub fn extract_bytes(&mut self, count: usize) -> Option<Vec<u8>> {
        if self
            .buffer
            .len()
            < count
        {
            return None;
        }
        let result = self.buffer[..count].to_vec();
        self.buffer
            .advance(count);
        self.scan_offset = self
            .scan_offset
            .saturating_sub(count);
        Some(result)
    }

    /// Ensure minimum write capacity; BytesMut handles internal compaction.
    pub fn compact(&mut self) {
        if self
            .buffer
            .remaining_mut()
            < BUF_CHUNK
        {
            self.buffer
                .reserve(BUF_CHUNK);
        }
    }

    /// Check if buffer size exceeds reasonable limits
    pub fn check_size_limits(&self) -> EslResult<()> {
        if self
            .buffer
            .len()
            > MAX_BUFFER_SIZE
        {
            tracing::error!(
                "Buffer overflow: {} bytes accumulated (limit {}). Memory leak or protocol desync.",
                self.buffer
                    .len(),
                MAX_BUFFER_SIZE
            );
            return Err(EslError::BufferOverflow {
                size: self
                    .buffer
                    .len(),
                limit: MAX_BUFFER_SIZE,
            });
        }
        Ok(())
    }
}

impl Default for EslBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let mut buffer = EslBuffer::new();
        assert_eq!(buffer.len(), 0);

        buffer.extend_from_slice(b"Hello World");
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.data(), b"Hello World");
    }

    #[test]
    fn test_advance() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Hello World");

        buffer
            .advance(6)
            .unwrap();
        assert_eq!(buffer.data(), b"World");
        assert_eq!(buffer.len(), 5);
    }

    #[test]
    fn test_advance_overflow() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Hello");
        assert!(buffer
            .advance(10)
            .is_err());
    }

    #[test]
    fn test_find_pattern() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Header1: Value1\r\nHeader2: Value2\r\n\r\nBody");

        let pos = buffer.find_pattern(b"\r\n\r\n");
        assert_eq!(pos, Some(32));
    }

    #[test]
    fn test_extract_until_pattern() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Header1: Value1\r\nHeader2: Value2\r\n\r\nBody");

        let headers = buffer
            .extract_until_pattern(b"\r\n\r\n")
            .unwrap();
        assert_eq!(headers, b"Header1: Value1\r\nHeader2: Value2");
        assert_eq!(buffer.data(), b"Body");
    }

    #[test]
    fn test_extract_bytes() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Hello World");

        let data = buffer
            .extract_bytes(5)
            .unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(buffer.data(), b" World");
    }

    #[test]
    fn test_compact() {
        let mut buffer = EslBuffer::new();
        buffer.extend_from_slice(b"Hello World");
        buffer
            .advance(6)
            .unwrap();

        assert_eq!(buffer.data(), b"World");
        buffer.compact();
        assert_eq!(buffer.data(), b"World");
    }

    #[test]
    fn buffer_exceeding_max_size_returns_error() {
        let mut buffer = EslBuffer::new();
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..17 {
            buffer.extend_from_slice(&chunk);
        }
        assert!(buffer.len() > MAX_BUFFER_SIZE);
        let err = buffer
            .check_size_limits()
            .unwrap_err();
        assert!(
            matches!(err, crate::EslError::BufferOverflow { .. }),
            "expected BufferOverflow, got: {err}"
        );
    }

    /// Pattern spanning the watermark boundary must still be found when
    /// data arrives in two chunks split inside the pattern.
    #[test]
    fn test_find_pattern_straddling_watermark() {
        let mut buffer = EslBuffer::new();
        // First chunk ends mid-pattern: \r\n\r is the start but \n is missing
        buffer.extend_from_slice(b"hello\r\n\r");
        assert_eq!(buffer.find_pattern(b"\r\n\r\n"), None);

        // Second chunk completes the pattern
        buffer.extend_from_slice(b"\nworld");
        // Must find the pattern at position 5, not miss it due to watermark skip
        assert_eq!(buffer.find_pattern(b"\r\n\r\n"), Some(5));
    }
}
