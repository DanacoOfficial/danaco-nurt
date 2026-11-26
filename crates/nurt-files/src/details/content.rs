// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content fields and details of `info`.
//!
//! The media type, binary flag, encoding and line endings of an entry come
//! from the leading bytes of the file ([`PREFIX_LEN`]); the line count and
//! the `details` groups read the content they need. The media type is taken
//! from the signature of the content, then, for text, from the file name,
//! and is `text/plain` or `application/octet-stream` otherwise.

use std::io::{self, BufReader, Read};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Fs};
use nurt_contracts::services::{LineEnding, TextCodecService, TextEncoding};
use nurt_contracts::value::{Fields, Value};
use serde_json::Value as Json;

use crate::commands::lines::splits_at_cr;
use crate::read::image::inspect;
use crate::read::pdf::PdfDocument;

/// Number of leading bytes read to recognise the content.
pub(crate) const PREFIX_LEN: u64 = 8192;

/// Media type of binary content without a known signature.
const OCTET_STREAM: &str = "application/octet-stream";

/// Media type of text without a type known from its name.
const TEXT_PLAIN: &str = "text/plain";

/// Media type of PDF documents.
pub(crate) const PDF_MIME: &str = "application/pdf";

/// Top-level media type of images.
const IMAGE_PREFIX: &str = "image/";

/// Top-level media types a file name may give to text content.
const TEXT_TYPES: [&str; 2] = ["text/", "application/"];

/// Longest UTF-8 sequence that a prefix may cut.
const MAX_UTF8_TAIL: usize = 3;

/// Properties of the content recognised from its leading bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sniffed {
    pub(crate) mime: String,
    pub(crate) binary: bool,
    pub(crate) encoding: Option<TextEncoding>,
    pub(crate) line_ending: Option<LineEnding>,
}

impl Sniffed {
    /// Returns `true` for an image.
    pub(crate) fn is_image(&self) -> bool {
        self.mime.starts_with(IMAGE_PREFIX)
    }
}

/// Properties of a whole text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextFacts {
    pub(crate) encoding: TextEncoding,
    pub(crate) line_ending: LineEnding,
    pub(crate) lines: Option<u64>,
}

/// `E_IO` for a failed read of a file.
pub(crate) fn read_error(path: &AbsPath, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("cannot read `{path}`: {error}"))
        .with_detail("path", Json::from(path.to_language_string()))
}

/// Leading bytes of a file, up to [`PREFIX_LEN`].
pub(crate) fn read_prefix(fs: &dyn Fs, path: &AbsPath) -> Result<Vec<u8>> {
    read_up_to(fs, path, PREFIX_LEN)
}

/// Whole content of a file of `size` bytes; `None` when it is larger than
/// `limit`.
pub(crate) fn read_whole(
    fs: &dyn Fs,
    path: &AbsPath,
    size: u64,
    limit: u64,
) -> Result<Option<Vec<u8>>> {
    if size > limit {
        return Ok(None);
    }
    read_up_to(fs, path, limit).map(Some)
}

fn read_up_to(fs: &dyn Fs, path: &AbsPath, limit: u64) -> Result<Vec<u8>> {
    let handle = fs.open_read(path)?;
    let mut bytes = Vec::new();
    handle
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| read_error(path, &error))?;
    Ok(bytes)
}

/// Recognises the content from its leading bytes; `complete` tells that
/// the prefix holds the whole file.
pub(crate) fn sniff(
    path: &AbsPath,
    prefix: &[u8],
    complete: bool,
    codec: &dyn TextCodecService,
) -> Sniffed {
    let sample = if complete {
        prefix
    } else {
        without_partial_utf8(prefix)
    };
    let detection = codec.detect(sample);
    let signature = infer::get(prefix).map(|kind| kind.mime_type());
    let mime = match signature {
        Some(mime) => mime.to_owned(),
        None if detection.binary => OCTET_STREAM.to_owned(),
        None => mime_guess::from_path(path.as_path())
            .first_raw()
            .filter(|mime| TEXT_TYPES.iter().any(|prefix| mime.starts_with(prefix)))
            .unwrap_or(TEXT_PLAIN)
            .to_owned(),
    };
    Sniffed {
        mime,
        binary: detection.binary,
        encoding: detection.encoding.filter(|_| !detection.binary),
        line_ending: (!detection.binary).then_some(detection.line_ending),
    }
}

/// The prefix without a UTF-8 sequence cut at its end, so that a cut
/// character does not make UTF-8 text look like another encoding.
fn without_partial_utf8(prefix: &[u8]) -> &[u8] {
    match std::str::from_utf8(prefix) {
        Err(error)
            if error.error_len().is_none()
                && prefix.len() - error.valid_up_to() <= MAX_UTF8_TAIL =>
        {
            &prefix[..error.valid_up_to()]
        }
        _ => prefix,
    }
}

/// Encoding, line endings and number of lines of a whole text; `None` for
/// binary content. The number of lines is `None` when the text cannot be
/// decoded in the detected encoding.
pub(crate) fn text_facts(codec: &dyn TextCodecService, bytes: &[u8]) -> Option<TextFacts> {
    let detection = codec.detect(bytes);
    if detection.binary {
        return None;
    }
    let encoding = detection.encoding.unwrap_or_else(TextEncoding::utf8);
    let lines = codec
        .decode(bytes, &encoding)
        .ok()
        .map(|text| count_lines(&text));
    Some(TextFacts {
        encoding,
        line_ending: detection.line_ending,
        lines,
    })
}

/// Number of lines of a text, split as `read` and `lines` split it: every
/// `\n` ends a line, with a `\r` before it, and a text with `\r` and no `\n`
/// ends its lines at `\r`. A last line without a terminator counts as well.
pub(crate) fn count_lines(text: &str) -> u64 {
    let separator = if splits_at_cr(text) { '\r' } else { '\n' };
    let ended = u64::try_from(text.matches(separator).count()).unwrap_or(u64::MAX);
    ended.saturating_add(u64::from(!text.is_empty() && !text.ends_with(separator)))
}

/// EXIF tags of the primary image, by tag name, with values in their
/// display form; `None` when the image carries no EXIF data.
pub(crate) fn exif_fields(fs: &dyn Fs, path: &AbsPath) -> Option<Fields> {
    let handle = fs.open_read(path).ok()?;
    let mut reader = BufReader::new(handle);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let mut fields = Fields::new();
    for field in exif
        .fields()
        .filter(|field| field.ifd_num == exif::In::PRIMARY)
    {
        fields
            .entry(field.tag.to_string())
            .or_insert_with(|| Value::Text(field.display_value().with_unit(&exif).to_string()));
    }
    (!fields.is_empty()).then_some(fields)
}

/// Version, page count and document information of a PDF document; `None`
/// when the bytes are not a readable PDF document.
pub(crate) fn pdf_fields(bytes: &[u8]) -> Option<Fields> {
    let metadata = PdfDocument::parse(bytes).ok()?.metadata();
    let mut fields = Fields::new();
    fields.insert("version".to_owned(), Value::Text(metadata.version));
    fields.insert("pages".to_owned(), Value::Int(i64::from(metadata.pages)));
    for (name, value) in [
        ("title", metadata.title),
        ("author", metadata.author),
        ("subject", metadata.subject),
        ("keywords", metadata.keywords),
        ("creator", metadata.creator),
        ("producer", metadata.producer),
    ] {
        if let Some(value) = value {
            fields.insert(name.to_owned(), Value::Text(value));
        }
    }
    Some(fields)
}

/// Media type and dimensions of an image; `None` for other content and
/// for a damaged image header.
pub(crate) fn media_fields(fs: &dyn Fs, path: &AbsPath) -> Option<Fields> {
    let handle = fs.open_read(path).ok()?;
    let info = inspect(BufReader::new(handle)).ok()?;
    let mut fields = Fields::new();
    fields.insert("mime".to_owned(), Value::Text(info.mime().to_owned()));
    if let (Some(width), Some(height)) = (info.width(), info.height()) {
        fields.insert("width".to_owned(), Value::Int(i64::from(width)));
        fields.insert("height".to_owned(), Value::Int(i64::from(height)));
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;
    use nurt_text::TextCodec;

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    /// JPEG with an EXIF segment holding one tag, `Orientation` = 1.
    pub(crate) fn jpeg_with_orientation() -> Vec<u8> {
        let tiff: [u8; 26] = [
            b'I', b'I', 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0,
            0, 0,
        ];
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1, 0, 34];
        bytes.extend_from_slice(b"Exif\0\0");
        bytes.extend_from_slice(&tiff);
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn lines_count_every_terminator_and_an_open_last_line() {
        let cases = [
            ("", 0),
            ("a", 1),
            ("a\n", 1),
            ("a\nb", 2),
            ("a\r\nb\r\n", 2),
            ("a\rb\n\n", 2),
            ("a\rb", 2),
            ("a\r\rb\r", 3),
            ("\n\n", 2),
        ];
        for (text, expected) in cases {
            assert_eq!(count_lines(text), expected, "{text:?}");
        }
    }

    #[test]
    fn text_is_typed_by_name_and_binary_by_signature() {
        let codec = TextCodec::default();
        let text = sniff(&abs("/data/notes.md"), b"# Title\r\n", true, &codec);
        assert!(!text.binary);
        assert_eq!(text.mime, "text/markdown");
        assert_eq!(text.line_ending, Some(LineEnding::CrLf));
        assert_eq!(
            text.encoding.map(|encoding| encoding.label),
            Some("utf-8".to_owned())
        );

        let video_name = sniff(&abs("/data/main.ts"), b"let a = 1;\n", true, &codec);
        assert_eq!(video_name.mime, TEXT_PLAIN);

        let image = sniff(&abs("/data/a.bin"), &png(1, 1), true, &codec);
        assert_eq!(image.mime, "image/png");
        assert!(image.is_image());

        let unknown = sniff(&abs("/data/a.dat"), &[0, 1, 2, 0, 0, 0xff], true, &codec);
        assert!(unknown.binary);
        assert_eq!(unknown.mime, OCTET_STREAM);
        assert_eq!(unknown.encoding, None);
        assert_eq!(unknown.line_ending, None);
    }

    #[test]
    fn a_cut_character_does_not_change_the_encoding() {
        let text = "\u{17c}\u{f3}\u{142}w".as_bytes();
        let cut = &text[..text.len() - 2];
        assert_eq!(without_partial_utf8(cut).len(), cut.len() - 1);
        assert_eq!(without_partial_utf8(text), text);
        assert_eq!(without_partial_utf8(&[0xff, b'a']), &[0xff, b'a']);
        let sniffed = sniff(&abs("/data/a.txt"), cut, false, &TextCodec::default());
        assert_eq!(
            sniffed.encoding.map(|encoding| encoding.label),
            Some("utf-8".to_owned())
        );
    }

    #[test]
    fn text_facts_hold_lines_and_skip_binary() {
        let codec = TextCodec::default();
        let facts = text_facts(&codec, b"one\ntwo\nthree").unwrap();
        assert_eq!(facts.lines, Some(3));
        assert_eq!(facts.line_ending, LineEnding::Lf);
        assert_eq!(text_facts(&codec, &[0, 0, 0, 1, 2]), None);
    }

    #[test]
    fn whole_reads_respect_the_limit() {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"12345").unwrap();
        let fs = test.fs().as_ref();
        assert_eq!(read_whole(fs, &path, 5, 4).unwrap(), None);
        assert_eq!(
            read_whole(fs, &path, 5, 5).unwrap(),
            Some(b"12345".to_vec())
        );
        assert_eq!(read_prefix(fs, &path).unwrap(), b"12345");
        let missing = read_prefix(fs, &abs("/data/b.txt")).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::NotFound);
    }

    #[test]
    fn images_give_dimensions_and_exif_tags() {
        let test = TestContext::new();
        let picture = abs("/data/picture.png");
        test.fs().write_file(&picture, &png(4, 3)).unwrap();
        let media = media_fields(test.fs().as_ref(), &picture).unwrap();
        assert_eq!(media.get("width"), Some(&Value::Int(4)));
        assert_eq!(media.get("height"), Some(&Value::Int(3)));
        assert_eq!(exif_fields(test.fs().as_ref(), &picture), None);

        let photo = abs("/data/photo.jpg");
        test.fs()
            .write_file(&photo, &jpeg_with_orientation())
            .unwrap();
        let exif = exif_fields(test.fs().as_ref(), &photo).unwrap();
        assert!(exif.contains_key("Orientation"), "{exif:?}");

        let text = abs("/data/a.txt");
        test.fs().write_file(&text, b"text").unwrap();
        assert_eq!(media_fields(test.fs().as_ref(), &text), None);
    }

    #[test]
    fn foreign_bytes_have_no_pdf_fields() {
        assert_eq!(pdf_fields(b"%PDF-1.7 not really"), None);
        assert_eq!(pdf_fields(b"plain text"), None);
    }
}
