// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading text files and writing changed text in their original form.
//!
//! A command that changes text reads the file with [`read_text`], computes
//! the new text, and writes it with [`write_text_preserving`]: the encoding,
//! the byte order mark and the line endings of the original are kept, so
//! only the intended change reaches the file. A character the encoding
//! cannot represent fails with `E_ENCODING` before anything is written.

use std::io::Read;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag, FileMeta, Follow, Fs};
use nurt_contracts::journal::{
    Applied, Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::{self, Change, ChangeAction};
use nurt_contracts::services::{LineEnding, TextCodecService, TextEncoding};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Size;
use nurt_text::TextCodec;
use nurt_text::codec::{decode, encode};
use nurt_text::codec_service::line_ending;
use nurt_text::eol;
use nurt_text::hash::content_hash;

use super::params::EncodingParam;
use super::path::path_field;

/// Text file read for a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    /// File.
    pub path: AbsPath,
    /// Metadata of the entry itself, a link in the last component not
    /// followed; the source of the entity tag.
    pub meta: FileMeta,
    /// Size of the content in bytes.
    pub size: u64,
    /// SHA-256 hash of the bytes read.
    pub hash: ContentHash,
    /// Decoded text, without the byte order mark.
    pub text: String,
    /// Encoding of the file.
    pub encoding: TextEncoding,
    /// Line endings of the file.
    pub line_ending: LineEnding,
}

impl TextFile {
    /// Entity tag of the file as read, with the hash of the content read.
    #[must_use]
    pub fn etag(&self) -> Etag {
        Etag {
            hash: Some(self.hash),
            ..self.meta.etag()
        }
    }

    /// Declaration of the file as a modified target that must still have the
    /// tag it had when it was read.
    #[must_use]
    pub fn target(&self) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Fs(self.path.clone()),
            action: TargetAction::Modify,
            expect: Expect::Etag(self.etag()),
            size_hint: Some(self.size),
        }
    }
}

/// Codec with the legacy encodings of the configuration key
/// `text.legacy_encodings`.
#[must_use]
pub fn text_codec(ctx: &ExecCtx) -> TextCodec {
    TextCodec::with_legacy_encodings(&ctx.config().text.legacy_encodings)
}

/// Reads a whole file as text.
///
/// With [`EncodingParam::Auto`] the encoding is detected; with a named
/// encoding the file is decoded in it, and a byte order mark of that
/// encoding at the start is recognised.
///
/// # Errors
///
/// Fails with `E_STAGE_FAILED` when the context has no file system, with the
/// errors of the file system (`E_NOT_FOUND`, `E_PERMISSION`, `E_IO`), with
/// `E_LIMIT` when the file is larger than
/// [`Limits::max_file_bytes_text`](nurt_contracts::stage::Limits), with
/// `E_BINARY` when detection finds binary content and with `E_ENCODING` when
/// the content is not valid in the encoding.
pub fn read_text(ctx: &ExecCtx, path: &AbsPath, encoding: EncodingParam) -> Result<TextFile> {
    let fs = ctx.require_fs()?;
    let meta = fs.metadata(path, Follow::NoFollow)?;
    let bytes = read_limited(fs, path, ctx.limits().max_file_bytes_text)?;
    let encoding = match encoding {
        EncodingParam::Auto => text_codec(ctx)
            .detect(&bytes)
            .encoding
            .ok_or_else(|| binary_error(path))?,
        EncodingParam::Codec(codec) => {
            codec.text_encoding(codec.bom().is_some_and(|bom| bytes.starts_with(bom)))
        }
    };
    let text = decode(&bytes, &encoding)
        .map_err(|error| error.into_error(Some(&path.to_language_string())))?;
    Ok(TextFile {
        path: path.clone(),
        meta,
        size: byte_count(bytes.len()),
        hash: content_hash(&bytes),
        line_ending: line_ending(&bytes, &encoding),
        text,
        encoding,
    })
}

fn read_limited(fs: &dyn Fs, path: &AbsPath, limit: u64) -> Result<Vec<u8>> {
    let mut handle = fs.open_read(path)?;
    let size = handle.meta()?.size;
    if size > limit {
        return Err(limit_error(path, size, limit));
    }
    let mut bytes = Vec::new();
    if let Some(mapped) = handle.as_bytes() {
        bytes.extend_from_slice(mapped);
    } else {
        handle
            .by_ref()
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
            })?;
    }
    let read = byte_count(bytes.len());
    if read > limit {
        return Err(limit_error(path, read, limit));
    }
    Ok(bytes)
}

fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

fn limit_error(path: &AbsPath, size: u64, limit: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "`{path}` has {size} bytes, more than the {limit} bytes read as text: read a range of lines instead"
        ),
    )
    .with_expected(serde_json::json!(limit))
    .with_actual(serde_json::json!(size))
}

fn binary_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Binary,
        format!("`{path}` has binary content, not text"),
    )
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

/// Encodes new text of a file read with [`read_text`] in the form of the
/// original: its encoding and byte order mark, and its line endings as
/// restored by [`eol::preserve`].
///
/// # Errors
///
/// Fails with `E_ENCODING` at the first character the encoding of the
/// original cannot represent; the error carries the location in the new
/// text and a `convert` call as the fix.
pub fn encode_preserving(new_text: &str, original: &TextFile) -> Result<Vec<u8>> {
    let text = eol::preserve(&original.text, new_text);
    encode(&text, &original.encoding)
        .map_err(|error| error.into_error(Some(&original.path.to_language_string())))
}

/// Writes new text of a file read with [`read_text`] through the
/// transaction, in the form of the original ([`encode_preserving`]).
///
/// `target` is the index of the file declared with [`TextFile::target`] (or
/// another declaration that writes content) in this transaction.
///
/// # Errors
///
/// Fails as [`encode_preserving`], before the transaction is used, and with
/// the errors of [`MutationTx::apply`], for example `E_STALE` when the file
/// changed since it was read.
pub fn write_text_preserving(
    tx: &mut dyn MutationTx,
    target: TargetIndex,
    new_text: &str,
    original: &TextFile,
) -> Result<Applied> {
    let bytes = encode_preserving(new_text, original)?;
    tx.apply(target, Mutation::WriteBytes(bytes))
}

/// `change` record of a text file rewritten with `written`, the bytes
/// returned by [`encode_preserving`]: sizes, encodings, line endings and byte
/// order marks before and after.
#[must_use]
pub fn text_change(original: &TextFile, written: &[u8]) -> Change {
    Change {
        size_before: Some(Size::from_bytes(original.size)),
        size_after: Some(Size::from_bytes(byte_count(written.len()))),
        encoding_before: Some(original.encoding.label.clone()),
        encoding_after: Some(original.encoding.label.clone()),
        line_ending_before: Some(record_line_ending(original.line_ending)),
        line_ending_after: Some(record_line_ending(line_ending(written, &original.encoding))),
        bom_before: Some(original.encoding.bom),
        bom_after: Some(original.encoding.bom),
        ..Change::new(path_field(&original.path), ChangeAction::Modify)
    }
}

/// Line endings as the value of a record field.
#[must_use]
pub fn record_line_ending(ending: LineEnding) -> record::LineEnding {
    match ending {
        LineEnding::Lf => record::LineEnding::Lf,
        LineEnding::CrLf => record::LineEnding::Crlf,
        LineEnding::Cr => record::LineEnding::Cr,
        LineEnding::None => record::LineEnding::None,
        _ => record::LineEnding::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::config::Config;
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::stage::Limits;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;
    use nurt_text::codec::Codec;

    use super::*;
    use crate::common::fixture::{abs, with_file};

    /// Windows-1250 text with Polish letters and CRLF line endings.
    const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn rewrite(test: &TestContext, file: &TextFile, new_text: &str) -> Result<Applied> {
        let mut tx = test.journal().begin(begin_request("replace"));
        let declared = tx.preserve(&[file.target()])?;
        let applied = write_text_preserving(tx.as_mut(), declared[0].index, new_text, file)?;
        tx.commit()?;
        Ok(applied)
    }

    #[test]
    fn detected_text_keeps_encoding_and_line_endings() {
        let (test, path) = with_file("/data/legacy.txt", CP1250_CRLF);
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        assert_eq!(file.encoding, Codec::Windows1250.text_encoding(false));
        assert_eq!(file.line_ending, LineEnding::CrLf);
        assert!(file.text.starts_with("Zielona \u{142}\u{105}ka"));
        assert_eq!(file.size, 40);

        let new_text = file.text.replace("koniec", "\u{17c}\u{f3}\u{142}w");
        rewrite(&test, &file, &new_text.replace("\r\n", "\n")).unwrap();
        let written = test.fs().read_file(&path).unwrap();
        assert_eq!(
            written,
            b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\n\xbf\xf3\xb3w\r\n"
        );
    }

    #[test]
    fn byte_order_mark_and_crlf_are_restored() {
        let (test, path) = with_file("/data/bom.txt", b"\xef\xbb\xbfone\r\ntwo\r\n");
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        assert_eq!(file.encoding, Codec::Utf8.text_encoding(true));
        assert_eq!(file.text, "one\r\ntwo\r\n");
        let bytes = encode_preserving("one\nTWO\nthree\n", &file).unwrap();
        assert_eq!(bytes, b"\xef\xbb\xbfone\r\nTWO\r\nthree\r\n");

        let change = text_change(&file, &bytes);
        assert_eq!(change.action, ChangeAction::Modify);
        assert_eq!(change.path.as_str(), path.to_language_string());
        assert_eq!(change.size_before, Some(Size::from_bytes(13)));
        assert_eq!(
            change.size_after,
            Some(Size::from_bytes(bytes.len() as u64))
        );
        assert_eq!(change.encoding_after.as_deref(), Some("utf-8"));
        assert_eq!(change.line_ending_before, Some(record::LineEnding::Crlf));
        assert_eq!(change.line_ending_after, Some(record::LineEnding::Crlf));
        assert_eq!(change.bom_after, Some(true));
    }

    #[test]
    fn utf16_files_are_decoded_and_encoded() {
        let mut content = vec![0xff, 0xfe];
        content.extend(utf16le("a\r\nb"));
        let (test, path) = with_file("/data/wide.txt", &content);
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        assert_eq!(file.encoding, Codec::Utf16Le.text_encoding(true));
        assert_eq!(file.line_ending, LineEnding::CrLf);
        assert_eq!(file.text, "a\r\nb");
        let mut expected = vec![0xff, 0xfe];
        expected.extend(utf16le("a\r\nc"));
        assert_eq!(encode_preserving("a\nc", &file).unwrap(), expected);
    }

    #[test]
    fn named_encoding_is_used_without_detection() {
        let (test, path) = with_file("/data/named.txt", b"\xef\xbb\xbfa\n");
        let as_utf8 = read_text(&test.ctx(), &path, EncodingParam::Codec(Codec::Utf8)).unwrap();
        assert_eq!(as_utf8.encoding, Codec::Utf8.text_encoding(true));
        assert_eq!(as_utf8.text, "a\n");
        let as_latin =
            read_text(&test.ctx(), &path, EncodingParam::Codec(Codec::Windows1252)).unwrap();
        assert_eq!(as_latin.encoding, Codec::Windows1252.text_encoding(false));
        assert_eq!(as_latin.text.chars().count(), 5);
    }

    #[test]
    fn invalid_content_in_a_named_encoding_fails() {
        let (test, path) = with_file("/data/bad.txt", b"ok \xff\n");
        let error = read_text(&test.ctx(), &path, EncodingParam::Codec(Codec::Utf8)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
        assert_eq!(
            error
                .location
                .as_ref()
                .map(|location| location.path.clone()),
            Some(path.to_language_string())
        );
    }

    #[test]
    fn binary_files_are_rejected() {
        let (test, path) = with_file("/data/image.bin", b"\x7fELF\x00\x01\x02\x00");
        let error = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Binary);
    }

    #[test]
    fn files_above_the_text_limit_are_rejected() {
        let (test, path) = with_file("/data/large.txt", b"0123456789");
        let limits = Limits {
            max_file_bytes_text: 4,
            ..Limits::default()
        };
        let ctx = test.builder().limits(limits).build();
        let error = read_text(&ctx, &path, EncodingParam::Auto).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
        assert_eq!(error.actual, Some(serde_json::json!(10)));
    }

    #[test]
    fn missing_files_fail_with_not_found() {
        let test = TestContext::new();
        let error =
            read_text(&test.ctx(), &abs("/data/none.txt"), EncodingParam::Auto).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn configured_legacy_encodings_drive_detection() {
        let text =
            "\u{179}r\u{f3}d\u{142}o: \u{17c}\u{f3}\u{142}w, g\u{119}\u{15b} i \u{142}\u{105}ka.\n";
        let bytes = encode(text, &Codec::Ibm852.text_encoding(false)).unwrap();
        let (test, path) = with_file("/data/dos.txt", &bytes);
        let mut config = Config::default();
        config.text.legacy_encodings = vec!["ibm852".to_owned()];
        let ctx = test.builder().config(Arc::new(config)).build();
        let file = read_text(&ctx, &path, EncodingParam::Auto).unwrap();
        assert_eq!(file.encoding.label, "ibm852");
        assert_eq!(file.text, text);
    }

    #[test]
    fn unrepresentable_text_is_not_written() {
        let (test, path) = with_file("/data/legacy.txt", CP1250_CRLF);
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        let error = rewrite(&test, &file, "\u{4e2d}\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
        assert_eq!(error.fix.len(), 1);
        assert_eq!(test.fs().read_file(&path).unwrap(), CP1250_CRLF);
        let applied = test
            .journal()
            .calls()
            .into_iter()
            .filter(|call| matches!(call, JournalCall::Apply { .. }))
            .count();
        assert_eq!(applied, 0);
    }

    #[test]
    fn changed_files_are_not_overwritten() {
        let (test, path) = with_file("/data/a.txt", b"first\n");
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        test.fs().write_file(&path, b"changed meanwhile\n").unwrap();
        let error = rewrite(&test, &file, "second\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"changed meanwhile\n");
    }

    #[test]
    fn targets_expect_the_tag_read() {
        let (test, path) = with_file("/data/a.txt", b"x");
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        let target = file.target();
        assert_eq!(target.path, TargetPath::Fs(path));
        assert_eq!(target.action, TargetAction::Modify);
        assert_eq!(target.expect, Expect::Etag(file.etag()));
        assert_eq!(target.size_hint, Some(1));
    }

    #[test]
    fn line_endings_map_to_record_values() {
        assert_eq!(record_line_ending(LineEnding::Lf), record::LineEnding::Lf);
        assert_eq!(record_line_ending(LineEnding::Cr), record::LineEnding::Cr);
        assert_eq!(
            record_line_ending(LineEnding::Mixed),
            record::LineEnding::Mixed
        );
        assert_eq!(
            record_line_ending(LineEnding::None),
            record::LineEnding::None
        );
    }
}
