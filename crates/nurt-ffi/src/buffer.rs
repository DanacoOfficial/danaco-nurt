// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ffi::buffer
// Responsibility:   Buffers returned by the library (`nurt_buf`).
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Buffers returned by the library (`nurt_buf`).

use std::mem::ManuallyDrop;
use std::ptr;

/// Buffer of UTF-8 text or JSON returned by the library (`nurt_buf`).
///
/// The layout is the C structure `{ uint8_t *ptr; size_t len; size_t cap; }`.
/// `ptr` is null exactly when `len` is 0, and a `\0` byte follows the last
/// byte without being counted in `len`. A buffer filled by the library is
/// released with [`nurt_buf_free`](crate::nurt_buf_free); the type has no
/// destructor because the C caller owns the storage of the structure.
#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct NurtBuf {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

impl NurtBuf {
    /// Empty buffer: null pointer, length and capacity 0.
    pub const EMPTY: Self = Self {
        ptr: ptr::null_mut(),
        len: 0,
        cap: 0,
    };

    /// Takes ownership of `bytes` and appends the terminating `\0`.
    pub(crate) fn from_bytes(mut bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return Self::EMPTY;
        }
        let len = bytes.len();
        bytes.push(0);
        let mut bytes = ManuallyDrop::new(bytes);
        Self {
            ptr: bytes.as_mut_ptr(),
            len,
            cap: bytes.capacity(),
        }
    }

    /// Number of bytes, without the terminating `\0`.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer holds no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Content of the buffer, without the terminating `\0`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        if self.ptr.is_null() {
            return &[];
        }
        // SAFETY: The fields are private, so a non-null buffer was created by
        // `from_bytes` and owns `len + 1` initialised bytes at `ptr` until it
        // is released, which resets it to the empty buffer.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Content of the buffer as text, or `None` when it is not UTF-8.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }

    /// Releases the allocation and resets the buffer to [`NurtBuf::EMPTY`].
    pub(crate) fn release(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: A non-null buffer describes the allocation of a
            // `Vec<u8>` of `len + 1` bytes and capacity `cap` made by
            // `from_bytes`; it is released once because the buffer is reset
            // right after.
            drop(unsafe { Vec::from_raw_parts(self.ptr, self.len + 1, self.cap) });
        }
        *self = Self::EMPTY;
    }
}

impl Default for NurtBuf {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::NurtBuf;

    #[test]
    fn empty_bytes_give_null_buffer() {
        let buf = NurtBuf::from_bytes(Vec::new());
        assert_eq!(buf, NurtBuf::EMPTY);
        assert!(buf.is_empty());
        assert_eq!(buf.as_bytes(), b"");
    }

    #[test]
    fn content_is_followed_by_nul() {
        let mut buf = NurtBuf::from_bytes(b"{}".to_vec());
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.as_str(), Some("{}"));
        // SAFETY: The buffer owns `len + 1` bytes, see `from_bytes`.
        let terminator = unsafe { *buf.ptr.add(buf.len) };
        assert_eq!(terminator, 0);
        buf.release();
        assert_eq!(buf, NurtBuf::EMPTY);
        buf.release();
        assert_eq!(buf, NurtBuf::EMPTY);
    }

    #[test]
    fn invalid_utf8_has_no_text() {
        let mut buf = NurtBuf::from_bytes(vec![0xff, 0xfe]);
        assert_eq!(buf.as_str(), None);
        buf.release();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
