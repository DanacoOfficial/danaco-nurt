// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve::framing
// Responsibility:   Delimiting of messages on the byte streams of the server.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Delimiting of messages on the byte streams of the server.

use std::io::{self, BufRead, Read, Write};

/// Longest header line of the `Content-Length` framing, in bytes.
const MAX_HEADER_LINE: usize = 1024;
/// Header carrying the length of the body, compared without regard to case.
const CONTENT_LENGTH: &str = "Content-Length";

/// How messages are delimited on the input and output streams.
///
/// # Examples
///
/// ```
/// use nurt_cli::serve::Framing;
///
/// assert_eq!(Framing::default(), Framing::Ndjson);
/// assert_eq!(Framing::from_name("content-length"), Some(Framing::ContentLength));
/// assert_eq!(Framing::ContentLength.name(), "content-length");
/// assert_eq!(Framing::from_name("xml"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Framing {
    /// One message per line, terminated by a line feed (newline-delimited
    /// JSON). A carriage return before the line feed and empty lines are
    /// ignored.
    #[default]
    Ndjson,
    /// Header lines terminated by an empty line, then a body of exactly
    /// `Content-Length` bytes. Lines end with a carriage return and a line
    /// feed.
    ContentLength,
}

impl Framing {
    /// Every framing.
    pub const ALL: &'static [Self] = &[Self::Ndjson, Self::ContentLength];

    /// Name of the framing as given to `nurt serve --framing`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ndjson => "ndjson",
            Self::ContentLength => "content-length",
        }
    }

    /// Framing with the name `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|framing| framing.name() == name)
    }
}

/// One unit read from the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Frame {
    /// The bytes of one message.
    Message(Vec<u8>),
    /// A message longer than the limit; its bytes were skipped.
    TooLarge,
    /// Framing that cannot be read, with the reason.
    Malformed(String),
    /// The end of the input.
    End,
}

/// Reads the next frame; messages longer than `max_bytes` are skipped and
/// reported as [`Frame::TooLarge`].
pub(crate) fn read_frame(
    input: &mut impl BufRead,
    framing: Framing,
    max_bytes: usize,
) -> io::Result<Frame> {
    match framing {
        Framing::Ndjson => read_line_frame(input, max_bytes),
        Framing::ContentLength => read_content_frame(input, max_bytes),
    }
}

/// Writes one message and flushes the output.
pub(crate) fn write_frame(
    output: &mut impl Write,
    framing: Framing,
    body: &[u8],
) -> io::Result<()> {
    match framing {
        Framing::Ndjson => {
            output.write_all(body)?;
            output.write_all(b"\n")?;
        }
        Framing::ContentLength => {
            write!(output, "{CONTENT_LENGTH}: {}\r\n\r\n", body.len())?;
            output.write_all(body)?;
        }
    }
    output.flush()
}

fn read_line_frame(input: &mut impl BufRead, max_bytes: usize) -> io::Result<Frame> {
    // The line may carry a carriage return and a line feed after the message.
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(2);
    loop {
        let mut line = Vec::new();
        let read = input.by_ref().take(limit).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(Frame::End);
        }
        if line.last() != Some(&b'\n') && u64::try_from(line.len()).unwrap_or(u64::MAX) == limit {
            input.skip_until(b'\n')?;
            return Ok(Frame::TooLarge);
        }
        trim_line_end(&mut line);
        if line.len() > max_bytes {
            return Ok(Frame::TooLarge);
        }
        if !line.iter().all(u8::is_ascii_whitespace) {
            return Ok(Frame::Message(line));
        }
    }
}

/// Header block of one message.
enum Headers {
    /// The input ended before the block was complete.
    End,
    /// The block with the declared length or the reason it is unusable.
    Read(Result<usize, String>),
}

fn read_content_frame(input: &mut impl BufRead, max_bytes: usize) -> io::Result<Frame> {
    let length = match read_headers(input)? {
        Headers::End => return Ok(Frame::End),
        Headers::Read(Err(problem)) => return Ok(Frame::Malformed(problem)),
        Headers::Read(Ok(length)) => length,
    };
    if length > max_bytes {
        let skipped = io::copy(
            &mut input
                .by_ref()
                .take(u64::try_from(length).unwrap_or(u64::MAX)),
            &mut io::sink(),
        )?;
        return Ok(
            if usize::try_from(skipped).is_ok_and(|skipped| skipped == length) {
                Frame::TooLarge
            } else {
                Frame::End
            },
        );
    }
    let mut body = vec![0; length];
    match input.read_exact(&mut body) {
        Ok(()) => Ok(Frame::Message(body)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(Frame::End),
        Err(error) => Err(error),
    }
}

fn read_headers(input: &mut impl BufRead) -> io::Result<Headers> {
    let mut length = None;
    let mut problem = None;
    let mut started = false;
    loop {
        let mut line = Vec::new();
        let read = input
            .by_ref()
            .take(MAX_HEADER_LINE as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(Headers::End);
        }
        if line.last() != Some(&b'\n') {
            if read < MAX_HEADER_LINE {
                return Ok(Headers::End);
            }
            input.skip_until(b'\n')?;
            problem = Some(format!(
                "a header line is longer than {MAX_HEADER_LINE} bytes"
            ));
            started = true;
            continue;
        }
        trim_line_end(&mut line);
        if line.is_empty() {
            if started {
                break;
            }
            continue;
        }
        started = true;
        match header_length(&line) {
            Ok(Some(value)) => length = Some(value),
            Ok(None) => {}
            Err(reason) => problem = Some(reason),
        }
    }
    let result = match (problem, length) {
        (Some(problem), _) => Err(problem),
        (None, Some(length)) => Ok(length),
        (None, None) => Err(format!("the header `{CONTENT_LENGTH}` is missing")),
    };
    Ok(Headers::Read(result))
}

/// Value of a `Content-Length` header line; `None` for other headers.
fn header_length(line: &[u8]) -> Result<Option<usize>, String> {
    let text = std::str::from_utf8(line).map_err(|_| "a header line is not UTF-8".to_owned())?;
    let (name, value) = text
        .split_once(':')
        .ok_or_else(|| format!("the header line `{text}` has no `:`"))?;
    if !name.trim().eq_ignore_ascii_case(CONTENT_LENGTH) {
        return Ok(None);
    }
    value.trim().parse().map(Some).map_err(|_| {
        format!(
            "`{CONTENT_LENGTH}` must be a number of bytes; given `{}`",
            value.trim()
        )
    })
}

fn trim_line_end(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn frames(framing: Framing, input: &[u8], max_bytes: usize) -> Vec<Frame> {
        let mut input = Cursor::new(input.to_vec());
        let mut frames = Vec::new();
        loop {
            let frame = read_frame(&mut input, framing, max_bytes).unwrap();
            let end = frame == Frame::End;
            frames.push(frame);
            if end {
                return frames;
            }
        }
    }

    fn message(text: &str) -> Frame {
        Frame::Message(text.as_bytes().to_vec())
    }

    #[test]
    fn lines_are_messages_and_blank_lines_are_skipped() {
        let input = b"{\"a\":1}\r\n\n  \n{\"b\":2}\n{\"c\":3}";
        assert_eq!(
            frames(Framing::Ndjson, input, 64),
            vec![
                message("{\"a\":1}"),
                message("{\"b\":2}"),
                message("{\"c\":3}"),
                Frame::End
            ]
        );
    }

    #[test]
    fn long_lines_are_skipped_and_reading_continues() {
        let input = b"0123456789abcdef\n{}\n0123456\r\n";
        assert_eq!(
            frames(Framing::Ndjson, input, 6),
            vec![Frame::TooLarge, message("{}"), Frame::TooLarge, Frame::End]
        );
        assert_eq!(
            frames(Framing::Ndjson, b"123456\r\n", 6),
            vec![message("123456"), Frame::End]
        );
    }

    #[test]
    fn content_length_frames_carry_exact_bodies() {
        let input = b"Content-Length: 7\r\nContent-Type: application/json\r\n\r\n{\"a\":1}\r\ncontent-length:2\r\n\r\n{}";
        assert_eq!(
            frames(Framing::ContentLength, input, 64),
            vec![message("{\"a\":1}"), message("{}"), Frame::End]
        );
    }

    #[test]
    fn broken_headers_are_malformed() {
        let missing = frames(Framing::ContentLength, b"Content-Type: x\r\n\r\n", 64);
        assert!(matches!(&missing[0], Frame::Malformed(reason) if reason.contains("missing")));
        let invalid = frames(Framing::ContentLength, b"Content-Length: ten\r\n\r\n", 64);
        assert!(matches!(&invalid[0], Frame::Malformed(reason) if reason.contains("`ten`")));
        let no_colon = frames(Framing::ContentLength, b"Content-Length 2\r\n\r\n{}", 64);
        assert!(matches!(&no_colon[0], Frame::Malformed(reason) if reason.contains("no `:`")));
    }

    #[test]
    fn unreadable_header_lines_are_malformed() {
        let not_text = frames(Framing::ContentLength, b"\xff: 2\r\n\r\n", 64);
        assert!(matches!(&not_text[0], Frame::Malformed(reason) if reason.contains("UTF-8")));
        let mut long = vec![b'x'; MAX_HEADER_LINE + 10];
        long.extend_from_slice(b"\r\nContent-Length: 2\r\n\r\n{}");
        let long = frames(Framing::ContentLength, &long, 64);
        assert!(matches!(&long[0], Frame::Malformed(reason) if reason.contains("longer")));
    }

    #[test]
    fn oversized_and_truncated_bodies() {
        let input = b"Content-Length: 10\r\n\r\n0123456789Content-Length: 2\r\n\r\n{}";
        assert_eq!(
            frames(Framing::ContentLength, input, 4),
            vec![Frame::TooLarge, message("{}"), Frame::End]
        );
        assert_eq!(
            frames(Framing::ContentLength, b"Content-Length: 10\r\n\r\n01", 64),
            vec![Frame::End]
        );
        assert_eq!(
            frames(Framing::ContentLength, b"Content-Length: 10\r\n\r\n01", 4),
            vec![Frame::End]
        );
        assert_eq!(
            frames(Framing::ContentLength, b"Content-Length: 2\r\n", 4),
            vec![Frame::End]
        );
        assert_eq!(
            frames(Framing::ContentLength, b"Content-Len", 4),
            vec![Frame::End]
        );
    }

    #[test]
    fn the_limit_counts_bytes_not_characters() {
        let text = "{\"a\":\"\u{142}\u{1f600}\"}";
        let bytes = text.len();
        assert!(bytes > text.chars().count());
        let line = format!("{text}\n");
        assert_eq!(
            frames(Framing::Ndjson, line.as_bytes(), bytes),
            vec![message(text), Frame::End]
        );
        assert_eq!(
            frames(Framing::Ndjson, line.as_bytes(), bytes - 1),
            vec![Frame::TooLarge, Frame::End]
        );
        let framed = format!("Content-Length: {bytes}\r\n\r\n{text}");
        assert_eq!(
            frames(Framing::ContentLength, framed.as_bytes(), bytes),
            vec![message(text), Frame::End]
        );
        assert_eq!(
            frames(Framing::ContentLength, framed.as_bytes(), bytes - 1),
            vec![Frame::TooLarge, Frame::End]
        );
    }

    #[test]
    fn a_body_of_zero_bytes_is_an_empty_message() {
        assert_eq!(
            frames(Framing::ContentLength, b"Content-Length: 0\r\n\r\n", 8),
            vec![message(""), Frame::End]
        );
    }

    #[test]
    fn blank_lines_before_the_headers_and_repeated_lengths_are_tolerated() {
        let input = b"\r\n\r\nContent-Length: 5\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(
            frames(Framing::ContentLength, input, 8),
            vec![message("{}"), Frame::End]
        );
    }

    #[test]
    fn header_names_are_compared_without_regard_to_case_and_white_space() {
        let input = b"CONTENT-length :  2 \r\n\r\n{}";
        assert_eq!(
            frames(Framing::ContentLength, input, 8),
            vec![message("{}"), Frame::End]
        );
    }

    #[test]
    fn lines_with_bytes_that_are_not_utf8_are_passed_on() {
        assert_eq!(
            frames(Framing::Ndjson, b"\xff\xfe\n", 8),
            vec![Frame::Message(vec![0xff, 0xfe]), Frame::End]
        );
    }

    #[test]
    fn a_line_without_a_line_feed_at_the_end_of_the_input_is_a_message() {
        assert_eq!(
            frames(Framing::Ndjson, b"{}", 8),
            vec![message("{}"), Frame::End]
        );
        assert_eq!(frames(Framing::Ndjson, b"", 8), vec![Frame::End]);
        assert_eq!(frames(Framing::Ndjson, b"\n\r\n  \n", 8), vec![Frame::End]);
    }

    #[test]
    fn a_header_that_ends_the_input_is_the_end_of_the_input() {
        assert_eq!(frames(Framing::ContentLength, b"", 8), vec![Frame::End]);
        assert_eq!(
            frames(Framing::ContentLength, b"\r\n\r\n", 8),
            vec![Frame::End]
        );
    }

    #[test]
    fn the_content_length_header_is_written_with_the_byte_length() {
        let mut output = Vec::new();
        write_frame(&mut output, Framing::ContentLength, "\u{142}".as_bytes()).unwrap();
        assert_eq!(output, b"Content-Length: 2\r\n\r\n\xc5\x82");
        let mut line = Vec::new();
        write_frame(&mut line, Framing::Ndjson, b"{}").unwrap();
        assert_eq!(line, b"{}\n");
    }

    #[test]
    fn written_frames_are_read_back() {
        for framing in Framing::ALL {
            let mut output = Vec::new();
            write_frame(&mut output, *framing, b"{\"x\":1}").unwrap();
            write_frame(&mut output, *framing, b"[]").unwrap();
            assert_eq!(
                frames(*framing, &output, 64),
                vec![message("{\"x\":1}"), message("[]"), Frame::End]
            );
            assert_eq!(Framing::from_name(framing.name()), Some(*framing));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
