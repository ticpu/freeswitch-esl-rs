//! Buffer management for ESL protocol parsing

use crate::{
    constants::*,
    error::{EslError, EslResult},
};
use bytes::{BufMut, BytesMut};

/// Buffer wrapper for efficient ESL protocol parsing
pub struct EslBuffer {
    buffer: BytesMut,
    position: usize,
}

impl EslBuffer {
    /// Create new buffer with default capacity
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(BUF_CHUNK),
            position: 0,
        }
    }

    /// Get current length of data in buffer
    pub fn len(&self) -> usize {
        self.buffer
            .len()
            - self.position
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

    /// Get reference to current data
    pub fn data(&self) -> &[u8] {
        &self.buffer[self.position..]
    }

    /// Consume bytes from the front of buffer.
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
        self.position += count;
        Ok(())
    }

    /// Find position of pattern in buffer, starting from current position
    pub fn find_pattern(&self, pattern: &[u8]) -> Option<usize> {
        let data = self.data();
        if pattern.is_empty() || data.len() < pattern.len() {
            return None;
        }

        (0..=(data.len() - pattern.len())).find(|&i| data[i..i + pattern.len()] == *pattern)
    }

    /// Extract data up to (but not including) the pattern
    pub fn extract_until_pattern(&mut self, pattern: &[u8]) -> Option<Vec<u8>> {
        if let Some(pos) = self.find_pattern(pattern) {
            let result = self.data()[..pos].to_vec();
            // pos + pattern.len() <= self.len() is guaranteed by find_pattern
            let _ = self.advance(pos + pattern.len());
            Some(result)
        } else {
            None
        }
    }

    /// Extract exact number of bytes
    pub fn extract_bytes(&mut self, count: usize) -> Option<Vec<u8>> {
        if self.len() >= count {
            let result = self.data()[..count].to_vec();
            let _ = self.advance(count);
            Some(result)
        } else {
            None
        }
    }

    /// Compact buffer by removing consumed data
    pub fn compact(&mut self) {
        if self.position > 0 {
            let remaining_len = self.len();
            if remaining_len > 0 {
                // Move remaining data to front
                self.buffer
                    .copy_within(self.position.., 0);
            }
            self.buffer
                .truncate(remaining_len);
            self.position = 0;

            // Reserve more space if needed
            if self
                .buffer
                .capacity()
                < BUF_CHUNK
            {
                self.buffer
                    .reserve(BUF_CHUNK);
            }
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
}
