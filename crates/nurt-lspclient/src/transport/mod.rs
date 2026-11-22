// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Message framing over standard input and output.
//!
//! Every message is a header block followed by a UTF-8 JSON body. The header
//! block holds `Content-Length` (required) and `Content-Type` (ignored) and
//! ends with an empty line. A frame with a missing or excessive length, a
//! body that is not JSON or a header line longer than [`HEADER_LINE_MAX`] is
//! reported as [`Frame::Rejected`] and the reader continues with the next
//! frame, so one malformed message does not end the connection.
//!
//! # Examples
//!
//! ```
//! use nurt_lspclient::transport::{Frame, FrameReader, encode};
//! use serde_json::json;
//!
//! let bytes = encode(&json!({"jsonrpc": "2.0", "method": "exit"}));
//! let mut reader = FrameReader::new(bytes.as_slice());
//! assert!(matches!(reader.next_frame()?, Some(Frame::Message(_))));
//! assert!(reader.next_frame()?.is_none());
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io::{self, BufRead, BufReader, Read, Write};

use serde_json::Value;

/// Largest accepted message body, in bytes.
pub const MESSAGE_BYTES_MAX: usize = 64 * 1024 * 1024;

/// Largest accepted header line, in bytes.
pub const HEADER_LINE_MAX: usize = 8 * 1024;

const CONTENT_LENGTH: &str = "content-length";

/// Encodes a message as a frame.
#[must_use]
pub fn encode(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(body.as_bytes());
    frame
}

/// Why a frame was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The header block has no valid `Content-Length`.
    MissingLength,
    /// The announced body exceeds [`MESSAGE_BYTES_MAX`].
    TooLarge(usize),
    /// A header line exceeds [`HEADER_LINE_MAX`].
    HeaderTooLong,
    /// The body is not a JSON object.
    InvalidBody,
}

/// One frame read from the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// A decoded JSON object.
    Message(Value),
    /// A malformed frame that was skipped.
    Rejected(FrameError),
}

/// Reader of frames from a byte stream cut at arbitrary points.
#[derive(Debug)]
pub struct FrameReader<R> {
    reader: BufReader<R>,
}

enum Header {
    End,
    Line(String),
    TooLong,
}

impl<R: Read> FrameReader<R> {
    /// Reader over `reader`.
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
        }
    }

    /// Reads the next frame; `None` at the end of the stream, including a
    /// stream that ends inside a frame.
    ///
    /// # Errors
    ///
    /// Fails when reading the underlying stream fails.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        let mut length: Option<usize> = None;
        let mut invalid = false;
        let mut seen_header = false;
        loop {
            let line = match self.header_line()? {
                None => return Ok(None),
                Some(Header::End) if seen_header => break,
                Some(Header::End) => continue,
                Some(Header::TooLong) => {
                    invalid = true;
                    seen_header = true;
                    continue;
                }
                Some(Header::Line(line)) => line,
            };
            seen_header = true;
            if let Some((name, value)) = line.split_once(':')
                && name.trim().eq_ignore_ascii_case(CONTENT_LENGTH)
            {
                length = value.trim().parse().ok();
            }
        }
        if invalid {
            return Ok(Some(Frame::Rejected(FrameError::HeaderTooLong)));
        }
        let Some(length) = length else {
            return Ok(Some(Frame::Rejected(FrameError::MissingLength)));
        };
        if length > MESSAGE_BYTES_MAX {
            return Ok(Some(Frame::Rejected(FrameError::TooLarge(length))));
        }
        let mut body = vec![0; length];
        match self.reader.read_exact(&mut body) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        Ok(Some(match serde_json::from_slice::<Value>(&body) {
            Ok(value @ Value::Object(_)) => Frame::Message(value),
            _ => Frame::Rejected(FrameError::InvalidBody),
        }))
    }

    fn header_line(&mut self) -> io::Result<Option<Header>> {
        let mut line = Vec::new();
        let limit = u64::try_from(HEADER_LINE_MAX).unwrap_or(u64::MAX);
        let read = (&mut self.reader).take(limit).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') {
            if read < HEADER_LINE_MAX {
                return Ok(None);
            }
            self.skip_line()?;
            return Ok(Some(Header::TooLong));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        Ok(Some(if text.is_empty() {
            Header::End
        } else {
            Header::Line(text.to_owned())
        }))
    }

    fn skip_line(&mut self) -> io::Result<()> {
        loop {
            let buffer = self.reader.fill_buf()?;
            if buffer.is_empty() {
                return Ok(());
            }
            if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                self.reader.consume(position + 1);
                return Ok(());
            }
            let length = buffer.len();
            self.reader.consume(length);
        }
    }
}

/// Writer of frames.
#[derive(Debug)]
pub struct FrameWriter<W> {
    writer: W,
}

impl<W: Write> FrameWriter<W> {
    /// Writer over `writer`.
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Writes one message and flushes the stream.
    ///
    /// # Errors
    ///
    /// Fails when writing to the underlying stream fails, for example after
    /// the server closed its input.
    pub fn send(&mut self, message: &Value) -> io::Result<()> {
        self.writer.write_all(&encode(message))?;
        self.writer.flush()
    }
}
