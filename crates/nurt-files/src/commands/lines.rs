// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `lines` command: the lines of a file, or of input content, as `line`
//! records.
//!
//! The source form reads a file as text (the encoding is detected) and
//! produces one record per line in the requested range. The stage form does
//! the same for every input `entry` (directories are passed over, binary files
//! are skipped and counted) and for the text of every input `content` record.
//!
//! A line ends at a line feed, which may be preceded by a carriage return;
//! a text without any line feed but with carriage returns is split at the
//! carriage returns. A final line without a line ending is a line; a final
//! line ending does not start another line. `start` and `end` are byte
//! offsets in the file as stored, also for encodings other than UTF-8.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag};
use nurt_contracts::record::{Content, Entry, EntryType, Line, LineEnding};
use nurt_contracts::report::Notable;
use nurt_contracts::services::TextEncoding;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{self, Range, Record, RecordKind, Value};
use nurt_text::codec::{Codec, encode};
use nurt_text::hash::{HashAlgorithm, Hasher};

use crate::common::etag::etag_text;
use crate::common::notable::counted;
use crate::common::params::EncodingParam;
use crate::common::path::{path_arg, path_field, record_path};
use crate::common::text::{TextFile, read_text};

/// Name of the command.
const COMMAND: &str = "lines";
/// Name of the `path` parameter.
const PATH: &str = "path";
/// Name of the `range` parameter.
const RANGE: &str = "range";
/// Value of `range` that selects every line.
const ALL: &str = "all";
/// Label of UTF-8 in [`TextEncoding::label`].
const UTF8: &str = "utf-8";

/// Stage of the `lines` command.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Lines;

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Lines)]
}

impl StageImpl for Lines {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry, RecordKind::Content]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Lines {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let range = range_param(args)?;
        if input.is_none() {
            let path = path_arg(args, PATH, ctx)?.ok_or_else(|| missing_path(args))?;
            let file = read_text(ctx, &path, EncodingParam::Auto)?;
            let lines = FileLines::from_file(ctx, file, range);
            return Ok(StageOutput::records(RecordStream::new(lines.map(Ok))));
        }
        input.map_records(|records| {
            RecordStream::new(StageLines {
                input: records,
                ctx,
                range,
                current: None,
                binary_skipped: 0,
                done: false,
            })
        })
    }
}

fn missing_path(args: &BoundArgs) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{}` needs a file: give `{PATH}`", args.command()),
    )
}

/// Reads `range`: a range of line numbers from 1, a single line number, or
/// `all`; every line without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value of another type and with
/// `E_INVALID_VALUE` for a bound below 1.
pub(crate) fn range_param(args: &BoundArgs) -> Result<Range> {
    let range = match args.value(RANGE) {
        None | Some(Value::Null) => Range::full(),
        Some(Value::Range(range)) => *range,
        Some(Value::Int(line)) => Range::single(*line),
        Some(Value::Text(text) | Value::Enum(text)) if text.trim() == ALL => Range::full(),
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`{}` expects a range of lines such as 10..20 for `{RANGE}`",
                    args.command()
                ),
            ));
        }
    };
    let below_one = [range.start(), range.end()]
        .into_iter()
        .flatten()
        .any(|bound| bound < 1);
    if below_one {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("line numbers start at 1, not `{range}`: give a range such as 1..20"),
        )
        .with_found(range.to_string()));
    }
    Ok(range)
}

/// Entity tag of a file as written in records.
///
/// When the rest of the pipeline needs entity tags (a changing stage or a
/// check follows), the tag carries the SHA-256 hash of the content: of
/// `bytes` when given, otherwise as computed by the file system. Otherwise
/// the tag holds the metadata only.
pub(crate) fn record_etag(
    ctx: &ExecCtx,
    path: &AbsPath,
    mut etag: Etag,
    bytes: Option<&[u8]>,
) -> String {
    if ctx.demand().etag && etag.hash.is_none() {
        etag.hash = match bytes {
            Some(bytes) => sha256(bytes),
            None => ctx.fs().and_then(|fs| fs.content_hash(path).ok()),
        };
    }
    etag_text(&etag)
}

fn sha256(bytes: &[u8]) -> Option<ContentHash> {
    let mut hasher = Hasher::new(HashAlgorithm::Sha256);
    hasher.update(bytes);
    <[u8; 32]>::try_from(hasher.finalize().as_bytes())
        .ok()
        .map(ContentHash)
}

/// Converts byte offsets in decoded UTF-8 text to byte offsets in the file
/// the text was decoded from.
///
/// Offsets are converted fastest in increasing order; a smaller offset than
/// the previous one starts again from the beginning of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OffsetMapper {
    mapping: Mapping,
    decoded: usize,
    original: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mapping {
    /// UTF-8: the offset plus the length of a byte order mark.
    Shift(u64),
    /// Another encoding: the length of the encoded prefix plus the length of
    /// a byte order mark.
    Encoded { encoding: TextEncoding, base: u64 },
}

impl OffsetMapper {
    /// Offsets in a text that starts `shift` bytes into the file and is
    /// stored as UTF-8.
    pub(crate) fn shifted(shift: u64) -> Self {
        Self {
            mapping: Mapping::Shift(shift),
            decoded: 0,
            original: shift,
        }
    }

    /// Offsets in the whole text of a file stored in `encoding`; `None` for
    /// an encoding the codec does not know.
    pub(crate) fn for_file(encoding: &TextEncoding) -> Option<Self> {
        let codec = Codec::from_encoding(encoding).ok()?;
        let base = if encoding.bom {
            codec.bom().map_or(0, |bom| byte_count(bom.len()))
        } else {
            0
        };
        if encoding.label == UTF8 {
            return Some(Self::shifted(base));
        }
        Some(Self {
            mapping: Mapping::Encoded {
                encoding: TextEncoding {
                    label: encoding.label.clone(),
                    bom: false,
                },
                base,
            },
            decoded: 0,
            original: base,
        })
    }

    /// Offset in the file of the byte at `offset` in `text`; an offset
    /// inside a character is moved back to the start of the character.
    pub(crate) fn map(&mut self, text: &str, offset: usize) -> u64 {
        let (encoding, base) = match &self.mapping {
            Mapping::Shift(shift) => return shift + byte_count(offset),
            Mapping::Encoded { encoding, base } => (encoding, *base),
        };
        let mut target = offset.min(text.len());
        while !text.is_char_boundary(target) {
            target -= 1;
        }
        if target < self.decoded {
            self.decoded = 0;
            self.original = base;
        }
        if let Some(piece) = text.get(self.decoded..target) {
            let length = encode(piece, encoding).map_or(piece.len(), |bytes| bytes.len());
            self.original += byte_count(length);
            self.decoded = target;
        }
        self.original
    }
}

fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Line ending of one line and the position after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineBounds {
    /// Offset after the content of the line.
    pub(crate) end: usize,
    /// Offset of the next line.
    pub(crate) next: usize,
    /// Line ending of the line.
    pub(crate) ending: LineEnding,
}

/// Bounds of the line starting at `from`; `None` at the end of the text.
/// With `cr_only`, lines end at carriage returns.
pub(crate) fn line_bounds(text: &str, from: usize, cr_only: bool) -> Option<LineBounds> {
    let rest = text.get(from..).filter(|rest| !rest.is_empty())?;
    let separator = if cr_only { '\r' } else { '\n' };
    let Some(found) = rest.find(separator) else {
        return Some(LineBounds {
            end: text.len(),
            next: text.len(),
            ending: LineEnding::None,
        });
    };
    let at = from + found;
    if cr_only {
        return Some(LineBounds {
            end: at,
            next: at + 1,
            ending: LineEnding::Cr,
        });
    }
    let crlf = found > 0 && rest.as_bytes().get(found - 1) == Some(&b'\r');
    Some(LineBounds {
        end: if crlf { at - 1 } else { at },
        next: at + 1,
        ending: if crlf {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        },
    })
}

/// Whether a text is split at carriage returns: it has carriage returns and
/// no line feed.
pub(crate) fn splits_at_cr(text: &str) -> bool {
    !text.contains('\n') && text.contains('\r')
}

/// Lines of one text in a range, produced one at a time.
#[derive(Debug)]
struct FileLines {
    text: String,
    position: usize,
    no: u64,
    range: Range,
    cr_only: bool,
    path: value::Path,
    etag: String,
    encoding: Option<String>,
    offsets: Option<OffsetMapper>,
}

impl FileLines {
    fn from_file(ctx: &ExecCtx, file: TextFile, range: Range) -> Self {
        let etag = record_etag(ctx, &file.path, file.etag(), None);
        Self {
            cr_only: splits_at_cr(&file.text),
            offsets: OffsetMapper::for_file(&file.encoding),
            path: path_field(&file.path),
            encoding: Some(file.encoding.label.clone()),
            text: file.text,
            position: 0,
            no: 1,
            range,
            etag,
        }
    }

    fn from_content(content: Content, range: Range) -> Self {
        let first = content.range.and_then(Range::start).unwrap_or(1);
        let offsets = if content.range.is_none() {
            OffsetMapper::for_file(&TextEncoding {
                label: content.encoding.clone(),
                bom: content.bom,
            })
        } else {
            None
        };
        Self {
            cr_only: splits_at_cr(&content.text),
            text: content.text,
            position: 0,
            no: u64::try_from(first).unwrap_or(1),
            range,
            path: content.path,
            etag: content.etag,
            encoding: Some(content.encoding),
            offsets,
        }
    }

    fn past_range(&self) -> bool {
        self.range
            .end()
            .is_some_and(|end| i64::try_from(self.no).map_or(true, |no| no > end))
    }
}

impl Iterator for FileLines {
    type Item = Record;

    fn next(&mut self) -> Option<Record> {
        loop {
            if self.past_range() {
                return None;
            }
            let bounds = line_bounds(&self.text, self.position, self.cr_only)?;
            let start = self.position;
            let no = self.no;
            self.position = bounds.next;
            self.no += 1;
            if !i64::try_from(no).is_ok_and(|no| self.range.contains(no)) {
                continue;
            }
            let (start_offset, end_offset) = match &mut self.offsets {
                Some(offsets) => (
                    Some(offsets.map(&self.text, start)),
                    Some(offsets.map(&self.text, bounds.end)),
                ),
                None => (None, None),
            };
            let line = Line {
                path: self.path.clone(),
                no,
                text: self
                    .text
                    .get(start..bounds.end)
                    .unwrap_or_default()
                    .to_owned(),
                line_ending: bounds.ending,
                etag: self.etag.clone(),
                start: start_offset,
                end: end_offset,
                range_index: None,
                encoding: self.encoding.clone(),
            };
            return Some(Record::from(line));
        }
    }
}

/// Lines of the input records of the stage form.
struct StageLines<'a> {
    input: RecordStream<'a>,
    ctx: &'a ExecCtx,
    range: Range,
    current: Option<FileLines>,
    binary_skipped: u64,
    done: bool,
}

impl StageLines<'_> {
    fn open(&mut self, record: Record) -> Result<Option<FileLines>> {
        match record.kind {
            RecordKind::Content => {
                let content = Content::try_from(record).map_err(Error::from)?;
                Ok(Some(FileLines::from_content(content, self.range)))
            }
            RecordKind::Entry => self.open_entry(&record),
            other => Err(Error::new(
                ErrorCode::Type,
                format!("`{COMMAND}` takes `entry` or `content` records, not `{other}`"),
            )),
        }
    }

    fn open_entry(&mut self, record: &Record) -> Result<Option<FileLines>> {
        let kind = Entry::try_from(record.clone()).map(|entry| entry.kind);
        if matches!(kind, Ok(EntryType::Directory)) {
            return Ok(None);
        }
        let path = record_path(record, self.ctx)?;
        match read_text(self.ctx, &path, EncodingParam::Auto) {
            Ok(file) => Ok(Some(FileLines::from_file(self.ctx, file, self.range))),
            Err(error) if error.code() == ErrorCode::Binary => {
                self.binary_skipped += 1;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

impl Iterator for StageLines<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Result<Record>> {
        while !self.done {
            if let Some(line) = self.current.as_mut().and_then(Iterator::next) {
                return Some(Ok(line));
            }
            self.current = None;
            let opened = match self.input.next()? {
                Ok(record) => self.open(record),
                Err(error) => Err(error),
            };
            match opened {
                Ok(lines) => self.current = lines,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

impl Drop for StageLines<'_> {
    fn drop(&mut self) {
        if self.binary_skipped > 0 {
            self.ctx
                .note(counted(Notable::BinarySkipped, self.binary_skipped));
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Follow, Fs};
    use nurt_contracts::stage::{ArgValue, Demand, StageData, StageFrame};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;
    use nurt_text::codec::Codec;

    use super::*;
    use crate::common::entry::entry_record;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    fn args(path: Option<&AbsPath>, range: Option<Value>) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        if let Some(path) = path {
            args.insert(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            );
        }
        if let Some(range) = range {
            args.insert(RANGE, ArgValue::Value(range), false);
        }
        args
    }

    fn range(start: Option<i64>, end: Option<i64>) -> Value {
        Value::Range(Range::new(start, end).unwrap())
    }

    fn collect(output: StageOutput<'_>) -> Vec<Result<Line>> {
        let StageData::Records(stream) = output.data else {
            panic!("records expected");
        };
        stream
            .map(|item| item.map(|record| Line::try_from(record).unwrap()))
            .collect()
    }

    fn source(ctx: &ExecCtx, args: &BoundArgs) -> Result<Vec<Line>> {
        let output = Lines.execute(args, StageInput::none(), ctx)?;
        collect(output).into_iter().collect()
    }

    /// Number, text, line ending and byte offsets of a line.
    type Row<'a> = (u64, &'a str, LineEnding, Option<u64>, Option<u64>);

    fn summary(lines: &[Line]) -> Vec<Row<'_>> {
        lines
            .iter()
            .map(|line| {
                (
                    line.no,
                    line.text.as_str(),
                    line.line_ending,
                    line.start,
                    line.end,
                )
            })
            .collect()
    }

    #[test]
    fn source_form_produces_every_line_with_offsets() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\r\nthree");
        let lines = source(&test.ctx(), &args(Some(&path), None)).unwrap();
        assert_eq!(
            summary(&lines),
            [
                (1, "one", LineEnding::Lf, Some(0), Some(3)),
                (2, "two", LineEnding::Crlf, Some(4), Some(7)),
                (3, "three", LineEnding::None, Some(9), Some(14)),
            ]
        );
        assert!(
            lines
                .iter()
                .all(|line| line.encoding.as_deref() == Some("utf-8"))
        );
        assert_eq!(lines[0].path.as_str(), path.to_language_string());
    }

    #[test]
    fn ranges_select_lines() {
        let (test, path) = with_file("/data/a.txt", b"1\n2\n3\n4\n5\n");
        let ctx = test.ctx();
        let numbers = |value: Option<Value>| -> Vec<u64> {
            source(&ctx, &args(Some(&path), value))
                .unwrap()
                .iter()
                .map(|line| line.no)
                .collect()
        };
        assert_eq!(numbers(None), [1, 2, 3, 4, 5]);
        assert_eq!(numbers(Some(range(Some(2), Some(3)))), [2, 3]);
        assert_eq!(numbers(Some(range(Some(4), None))), [4, 5]);
        assert_eq!(numbers(Some(range(None, Some(2)))), [1, 2]);
        assert_eq!(numbers(Some(Value::Int(5))), [5]);
        assert_eq!(
            numbers(Some(Value::Text("all".to_owned()))),
            [1, 2, 3, 4, 5]
        );
        assert!(numbers(Some(range(Some(9), None))).is_empty());
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        let (test, path) = with_file("/data/a.txt", b"x\n");
        let ctx = test.ctx();
        let zero = source(&ctx, &args(Some(&path), Some(range(Some(0), Some(2))))).unwrap_err();
        assert_eq!(zero.code(), ErrorCode::InvalidValue);
        assert_eq!(zero.found.as_deref(), Some("0..2"));
        let wrong = source(&ctx, &args(Some(&path), Some(Value::Bool(true)))).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn missing_path_and_missing_file_fail() {
        let test = TestContext::new();
        let ctx = test.ctx();
        assert_eq!(
            source(&ctx, &args(None, None)).unwrap_err().code(),
            ErrorCode::MissingParam
        );
        let absent = abs("/data/none.txt");
        assert_eq!(
            source(&ctx, &args(Some(&absent), None)).unwrap_err().code(),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn legacy_encodings_report_offsets_in_the_file() {
        let (test, path) = with_file(
            "/data/legacy.txt",
            b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n",
        );
        let lines = source(&test.ctx(), &args(Some(&path), None)).unwrap();
        assert!(lines[0].text.starts_with("Zielona \u{142}\u{105}ka"));
        assert_eq!(lines[1].text, "koniec");
        assert_eq!((lines[0].start, lines[0].end), (Some(0), Some(30)));
        assert_eq!((lines[1].start, lines[1].end), (Some(32), Some(38)));
        assert_eq!(lines[1].line_ending, LineEnding::Crlf);
        assert_eq!(lines[0].encoding.as_deref(), Some("windows-1250"));
    }

    #[test]
    fn byte_order_mark_shifts_offsets() {
        let (test, path) = with_file("/data/bom.txt", b"\xef\xbb\xbfab\ncd\n");
        let lines = source(&test.ctx(), &args(Some(&path), None)).unwrap();
        assert_eq!(
            summary(&lines)[0],
            (1, "ab", LineEnding::Lf, Some(3), Some(5))
        );
        assert_eq!(
            summary(&lines)[1],
            (2, "cd", LineEnding::Lf, Some(6), Some(8))
        );
    }

    #[test]
    fn carriage_returns_split_lines_without_line_feeds() {
        let (test, path) = with_file("/data/mac.txt", b"a\rb\r");
        let lines = source(&test.ctx(), &args(Some(&path), None)).unwrap();
        assert_eq!(
            summary(&lines),
            [
                (1, "a", LineEnding::Cr, Some(0), Some(1)),
                (2, "b", LineEnding::Cr, Some(2), Some(3)),
            ]
        );
    }

    #[test]
    fn empty_files_have_no_lines() {
        let (test, path) = with_file("/data/empty.txt", b"");
        assert!(
            source(&test.ctx(), &args(Some(&path), None))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn etag_carries_a_hash_only_when_demanded() {
        let (test, path) = with_file("/data/a.txt", b"x\n");
        let with_hash = source(&test.ctx(), &args(Some(&path), None)).unwrap();
        assert!(with_hash[0].etag.contains("sha256:"));
        let frame = StageFrame {
            demand: Demand {
                etag: false,
                ..Demand::default()
            },
            ..StageFrame::default()
        };
        let ctx = test.builder().frame(frame).build();
        let plain = source(&ctx, &args(Some(&path), None)).unwrap();
        let meta = test.fs().metadata(&path, Follow::NoFollow).unwrap();
        assert_eq!(plain[0].etag, etag_text(&meta.etag()));
    }

    fn stage(ctx: &ExecCtx, args: &BoundArgs, records: Vec<Record>) -> Vec<Result<Line>> {
        let input = StageInput::records(RecordStream::from_records(records));
        collect(Lines.execute(args, input, ctx).unwrap())
    }

    #[test]
    fn content_records_keep_their_line_numbers() {
        let test = TestContext::new();
        let content = Content {
            path: path_field(&abs("/data/a.txt")),
            text: "ten\neleven\ntwelve\n".to_owned(),
            lines: 3,
            etag: "tag".to_owned(),
            encoding: "utf-8".to_owned(),
            range: Some(Range::new(Some(10), Some(12)).unwrap()),
            ..Content::default()
        };
        let lines = stage(
            &test.ctx(),
            &args(None, Some(range(Some(11), None))),
            vec![Record::from(content)],
        );
        let lines: Vec<Line> = lines.into_iter().map(Result::unwrap).collect();
        assert_eq!(
            summary(&lines),
            [
                (11, "eleven", LineEnding::Lf, None, None),
                (12, "twelve", LineEnding::Lf, None, None)
            ]
        );
        assert_eq!(lines[0].etag, "tag");
    }

    #[test]
    fn whole_content_records_have_offsets() {
        let test = TestContext::new();
        let content = Content {
            path: path_field(&abs("/data/a.txt")),
            text: "a\nb".to_owned(),
            etag: "tag".to_owned(),
            encoding: "utf-8".to_owned(),
            bom: true,
            ..Content::default()
        };
        let lines: Vec<Line> = stage(&test.ctx(), &args(None, None), vec![Record::from(content)])
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(lines[1].start, Some(5));
    }

    #[test]
    fn entries_are_read_and_binary_files_skipped() {
        let test = TestContext::new();
        let text = abs("/data/a.txt");
        let binary = abs("/data/b.bin");
        let dir = abs("/data");
        test.fs().write_file(&text, b"x\ny\n").unwrap();
        test.fs()
            .write_file(&binary, b"\x7fELF\x00\x01\x02\x00")
            .unwrap();
        let ctx = test.ctx();
        let entry = |path: &AbsPath| {
            let meta = test.fs().metadata(path, Follow::NoFollow).unwrap();
            Record::from(entry_record(path, &meta))
        };
        let lines = stage(
            &ctx,
            &args(None, None),
            vec![entry(&dir), entry(&binary), entry(&text)],
        );
        let numbers: Vec<u64> = lines.into_iter().map(|line| line.unwrap().no).collect();
        assert_eq!(numbers, [1, 2]);
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::BinarySkipped);
        assert_eq!(notes[0].count, Some(1));
    }

    #[test]
    fn other_records_end_the_stream_with_a_type_error() {
        let test = TestContext::new();
        let record = Record::new(RecordKind::Match)
            .with("path", Value::Path(abs("/x").to_language_string()));
        let items = stage(&test.ctx(), &args(None, None), vec![record.clone(), record]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn offsets_move_back_to_character_starts() {
        let encoding = Codec::Windows1250.text_encoding(false);
        let mut offsets = OffsetMapper::for_file(&encoding).unwrap();
        let text = "\u{142}a\u{105}b";
        assert_eq!(offsets.map(text, 2), 1);
        assert_eq!(offsets.map(text, 4), 2);
        assert_eq!(offsets.map(text, 1), 0);
        assert_eq!(offsets.map(text, 99), 4);
        let mut shifted = OffsetMapper::shifted(3);
        assert_eq!(shifted.map(text, 2), 5);
    }

    #[test]
    fn utf16_offsets_count_code_units() {
        let encoding = Codec::Utf16Le.text_encoding(true);
        let mut offsets = OffsetMapper::for_file(&encoding).unwrap();
        assert_eq!(offsets.map("ab\n", 2), 6);
    }

    #[test]
    fn descriptor_names_the_command_and_inputs() {
        let descriptor = Lines.descriptor();
        assert_eq!(descriptor.command, "lines");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Content));
        assert!(!descriptor.input.accepts(RecordKind::Line));
        assert_eq!(stages().len(), 1);
    }
}
