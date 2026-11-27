// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-net::client::body
// Responsibility:   Response bodies: decompression, size limits, progress and cancellation.
// Dependencies:     flate2, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Response bodies: decompression, size limits, progress and cancellation.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{CancelToken, Event, ExecCtx, Progress};

/// Interval at which the watcher of a connection checks for cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(20);
/// Size of the buffer used to read a body.
const CHUNK: usize = 16 * 1024;

/// Coding of a body as received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyEncoding {
    /// The body was sent without content coding.
    Identity,
    /// The body was sent compressed (`gzip` or `deflate`) and is decompressed
    /// while it is read.
    Decompressed(String),
    /// The body uses a content coding the client does not decode; the bytes
    /// are returned as received.
    Unsupported(String),
}

impl BodyEncoding {
    pub(crate) fn from_header(value: Option<&str>) -> Self {
        let coding = value.unwrap_or_default().trim().to_ascii_lowercase();
        match coding.as_str() {
            "" | "identity" => Self::Identity,
            "gzip" | "x-gzip" | "deflate" => Self::Decompressed(coding),
            _ => Self::Unsupported(coding),
        }
    }
}

/// A body read into memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitedBody {
    /// Bytes after decompression, at most the limit.
    pub bytes: Vec<u8>,
    /// The body had more bytes than the limit, or its compressed form broke
    /// after part of it was read; `bytes` holds the part read.
    pub truncated: bool,
}

/// Closes the connection of a request when its execution is cancelled.
pub(crate) struct CancelWatch {
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CancelWatch {
    pub(crate) fn start(agent: ureq::Agent, cancel: CancelToken) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                if cancel.wait_timeout(CANCEL_POLL).is_some() {
                    agent.shutdown();
                    return;
                }
            }
        });
        Self {
            done,
            handle: Some(handle),
        }
    }
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Body of a response, read after the head.
///
/// Reading checks the cancellation of the execution, reports progress
/// events every `progress_step` bytes and maps failures to `E_NETWORK`
/// (problems `reset` and `decode`), `E_TIMEOUT` and `E_CANCELLED`.
pub struct ResponseBody {
    reader: Box<dyn Read + Send>,
    encoding: BodyEncoding,
    content_length: Option<u64>,
    ctx: ExecCtx,
    url: String,
    received: u64,
    progress_step: u64,
    next_progress: u64,
    _watch: CancelWatch,
}

impl std::fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponseBody")
            .field("url", &self.url)
            .field("encoding", &self.encoding)
            .field("received", &self.received)
            .finish_non_exhaustive()
    }
}

pub(crate) struct BodySource {
    pub(crate) reader: ureq::BodyReader<'static>,
    pub(crate) encoding: BodyEncoding,
    pub(crate) content_length: Option<u64>,
    pub(crate) watch: CancelWatch,
}

impl ResponseBody {
    pub(crate) fn new(source: BodySource, ctx: &ExecCtx, url: String, progress_step: u64) -> Self {
        let reader: Box<dyn Read + Send> = match &source.encoding {
            BodyEncoding::Decompressed(coding) if coding == "deflate" => {
                deflate_decoder(source.reader)
            }
            _ => Box::new(source.reader),
        };
        let step = progress_step.max(1);
        Self {
            reader,
            encoding: source.encoding,
            content_length: source.content_length,
            ctx: ctx.clone(),
            url,
            received: 0,
            progress_step: step,
            next_progress: step,
            _watch: source.watch,
        }
    }

    /// Coding of the body as received.
    #[must_use]
    pub fn encoding(&self) -> &BodyEncoding {
        &self.encoding
    }

    /// Length announced by the server, before decompression.
    #[must_use]
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// Bytes read so far, after decompression.
    #[must_use]
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Reads the next bytes into `buf`; 0 at the end of the body.
    ///
    /// # Errors
    ///
    /// `E_CANCELLED` or `E_TIMEOUT` when the execution ends, `E_NETWORK` with
    /// problem `reset` when the connection breaks and `decode` when the
    /// compressed form is damaged.
    pub fn read_chunk(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.ctx.cancel().check()?;
        let read = self
            .reader
            .read(buf)
            .map_err(|error| self.read_error(&error))?;
        self.received += read as u64;
        if self.received >= self.next_progress {
            self.next_progress = self.received.saturating_add(self.progress_step);
            self.emit_progress();
        }
        Ok(read)
    }

    /// Reads the body into memory, keeping at most `max` bytes.
    ///
    /// # Errors
    ///
    /// As [`read_chunk`](Self::read_chunk); a damaged compressed form after
    /// part of the body was read ends the body with `truncated: true`
    /// instead.
    pub fn read_limited(&mut self, max: u64) -> Result<LimitedBody> {
        let max = usize::try_from(max).unwrap_or(usize::MAX);
        let mut bytes = Vec::new();
        let mut buf = vec![0_u8; CHUNK];
        loop {
            let read = match self.read_chunk(&mut buf) {
                Ok(read) => read,
                Err(error) if is_decode(&error) && !bytes.is_empty() => {
                    return Ok(LimitedBody {
                        bytes,
                        truncated: true,
                    });
                }
                Err(error) => return Err(error),
            };
            if read == 0 {
                return Ok(LimitedBody {
                    bytes,
                    truncated: false,
                });
            }
            let room = max.saturating_sub(bytes.len());
            if read > room {
                bytes.extend_from_slice(&buf[..room]);
                return Ok(LimitedBody {
                    bytes,
                    truncated: true,
                });
            }
            bytes.extend_from_slice(&buf[..read]);
        }
    }

    /// Copies the whole body to `out` and returns the number of bytes.
    ///
    /// # Errors
    ///
    /// As [`read_chunk`](Self::read_chunk), and `E_IO` when `out` fails.
    pub fn copy_to(&mut self, out: &mut dyn Write) -> Result<u64> {
        let mut buf = vec![0_u8; CHUNK];
        let mut total = 0_u64;
        loop {
            let read = self.read_chunk(&mut buf)?;
            if read == 0 {
                return Ok(total);
            }
            out.write_all(&buf[..read]).map_err(|error| {
                Error::new(ErrorCode::Io, format!("writing the body failed: {error}"))
            })?;
            total += read as u64;
        }
    }

    fn emit_progress(&self) {
        self.ctx.emit(Event::Progress(Progress {
            command: self.ctx.frame().command.clone(),
            phase: "receive".to_owned(),
            done: self.received,
            total: self.content_length,
            bytes: Some(self.received),
            message: Some(self.url.clone()),
        }));
    }

    fn read_error(&self, error: &io::Error) -> Error {
        if let Some(reason) = self.ctx.cancel().reason() {
            return reason.to_error();
        }
        let inner = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<ureq::Error>());
        let timed_out = matches!(inner, Some(ureq::Error::Timeout(_)))
            || error.kind() == io::ErrorKind::TimedOut;
        if timed_out {
            return Error::new(
                ErrorCode::Timeout,
                format!(
                    "reading the response of `{}` exceeded the time limit",
                    self.url
                ),
            );
        }
        let decode = matches!(inner, Some(ureq::Error::Decompress(..)))
            || error.kind() == io::ErrorKind::InvalidData
            || error.kind() == io::ErrorKind::InvalidInput;
        let problem = if decode { "decode" } else { "reset" };
        Error::new(
            ErrorCode::Network,
            format!("reading the response of `{}` failed: {error}", self.url),
        )
        .with_problem(problem)
    }
}

fn is_decode(error: &Error) -> bool {
    error.code() == ErrorCode::Network && error.data().problem.as_deref() == Some("decode")
}

/// Whether two bytes are the header of a zlib stream (RFC 1950): the method
/// `deflate` in the low bits of the first byte and a check value that makes
/// the two bytes a multiple of 31.
fn is_zlib_header(first: u8, second: u8) -> bool {
    first & 0x0f == 8 && (u16::from(first) << 8 | u16::from(second)) % 31 == 0
}

/// Decoder of a body sent with the coding `deflate`.
///
/// RFC 9110 defines the coding as the zlib format, but many servers send the
/// raw deflate stream without the zlib header; the first two bytes tell them
/// apart.
fn deflate_decoder<R: Read + Send + 'static>(source: R) -> Box<dyn Read + Send> {
    let mut buffered = io::BufReader::new(source);
    let zlib = match io::BufRead::fill_buf(&mut buffered) {
        Ok([first, second, ..]) => is_zlib_header(*first, *second),
        _ => true,
    };
    if zlib {
        Box::new(flate2::read::ZlibDecoder::new(buffered))
    } else {
        Box::new(flate2::read::DeflateDecoder::new(buffered))
    }
}

#[cfg(test)]
mod tests {
    use flate2::Compression;
    use flate2::write::{DeflateEncoder, ZlibEncoder};

    use super::*;

    fn decoded(bytes: Vec<u8>) -> String {
        let mut text = String::new();
        deflate_decoder(io::Cursor::new(bytes))
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn deflate_bodies_are_read_with_or_without_the_zlib_header() {
        let text = "a deflate body, a deflate body, a deflate body".repeat(8);
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(text.as_bytes()).unwrap();
        assert_eq!(decoded(zlib.finish().unwrap()), text);
        let mut raw = DeflateEncoder::new(Vec::new(), Compression::default());
        raw.write_all(text.as_bytes()).unwrap();
        assert_eq!(decoded(raw.finish().unwrap()), text);
    }

    #[test]
    fn classifies_content_codings() {
        assert_eq!(BodyEncoding::from_header(None), BodyEncoding::Identity);
        assert_eq!(
            BodyEncoding::from_header(Some(" GZIP ")),
            BodyEncoding::Decompressed("gzip".to_owned())
        );
        assert_eq!(
            BodyEncoding::from_header(Some("br")),
            BodyEncoding::Unsupported("br".to_owned())
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
