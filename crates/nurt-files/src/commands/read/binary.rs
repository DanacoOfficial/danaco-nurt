// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Content of `read` that is not text: pages of PDF documents, images,
//! hexadecimal dumps and compressed files read after decompression.
//!
//! A file is recognised by its leading bytes. With `decompress: auto` a file
//! whose name and signature agree on gzip, zstd, xz or bzip2 is decompressed
//! first, and the decompressed content is read as text, as a PDF document or
//! as an image; its records name the compressed file in `decompressed_from`.
//! `view: hex` dumps the bytes, after decompression when it applies, in rows
//! of 16 bytes; `lines`, `last` and `pages` select rows of the dump, counted
//! from 1.
//!
//! Content held in memory, decompressed content and the text of a dump are
//! limited to the text limit of the context (`E_LIMIT`). Records of content
//! that is not text have the encoding `binary` and no line endings; their
//! entity tag carries the content hash whenever the whole file was read.

use std::collections::VecDeque;
use std::io::{self, BufReader, Cursor, Read as _, Seek};
use std::ops::Bound;

use nurt_archive::codec::Algorithm;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, FileKind, FileMeta, Follow, Fs};
use nurt_contracts::record::{Content, ContentImage};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{LineEnding, TextCodecService, TextEncoding};
use nurt_contracts::stage::{BoundArgs, ExecCtx};
use nurt_contracts::value::{Bytes, Range, Record, RecordKind, Size, Value};
use nurt_text::codec::decode;
use nurt_text::codec_service::line_ending;
use nurt_text::hash::content_hash;

use super::cache::{MemoryReadCache, ReadCache};
use super::select::{AROUND, CELL, KEY, LAST, LINES, SECTION, SYMBOL, Selector};
use super::text::{self, ReadOptions, TextSource, View};
use crate::common::etag::etag_text;
use crate::common::notable::at_path;
use crate::common::params::EncodingParam;
use crate::common::path::path_field;
use crate::common::text::{record_line_ending, text_codec};
use crate::read::compressed::{self, DecompressError, DecompressedReader};
use crate::read::hex::{self, HexRow, HexRows};
use crate::read::image::{self, ImageError, ImageInfo};
use crate::read::pdf::{PdfDocument, PdfError, PdfTextMode};

const COMMAND: &str = "read";
const PAGES: &str = "pages";
const MODE: &str = "mode";
const DECOMPRESS: &str = "decompress";

/// Field of `content` records naming the compressed file.
const DECOMPRESSED_FROM: &str = "decompressed_from";

/// Encoding reported for content that is not text.
const BINARY_ENCODING: &str = "binary";

/// Values of `mode`, in the order of the catalog.
const MODES: [&str; 3] = ["text", "layout", "images"];
const MODE_LAYOUT: &str = "layout";
const MODE_IMAGES: &str = "images";

/// Values of `decompress`, in the order of the catalog.
const DECOMPRESS_VALUES: [&str; 2] = ["auto", "none"];
const DECOMPRESS_NONE: &str = "none";

/// Leading bytes of every PDF document.
const PDF_SIGNATURE: &[u8] = b"%PDF-";

/// Leading bytes read to recognise the kind of content; enough for images,
/// PDF documents and compressed files.
const SIGNATURE_LEN: u64 = image::SIGNATURE_LEN as u64;

/// Records of one file whose content is not text, or of any file with
/// `view: hex`.
///
/// `selector` and `options` are those of the target; `pages`, `mode` and
/// `decompress` are read from `args`.
///
/// # Errors
///
/// Fails with the errors of the file system (`E_NOT_FOUND`,
/// `E_PERMISSION`, `E_IO`), with `E_INVALID_VALUE` for a directory or a
/// special file and for page numbers below 1, with `E_BINARY` for binary
/// content of no supported kind, with `E_UNSUPPORTED` for a selector or view
/// that does not apply to the content, with `E_PARAM_CONFLICT` when rows of
/// a dump are selected twice, with `E_OUT_OF_RANGE` for a page outside the
/// document, with `E_FORMAT` and `E_ENCRYPTED` for unreadable documents,
/// images and compressed data, with `E_EXTERNAL_MISSING` for `mode: images`
/// and with `E_LIMIT` for content above the text limit.
pub(crate) fn read_binary(
    ctx: &ExecCtx,
    args: &BoundArgs,
    path: &AbsPath,
    selector: &Selector,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let params = Params::from_args(args)?;
    let fs = ctx.require_fs()?;
    let followed = fs.metadata(path, Follow::Follow)?;
    if followed.kind != FileKind::File {
        return Err(not_file_error(path, followed.kind));
    }
    let file = File {
        path,
        meta: fs.metadata(path, Follow::NoFollow)?,
        size: followed.size,
    };
    let prefix = read_up_to(fs, path, SIGNATURE_LEN)?;
    let compression = if params.decompress {
        compressed::compression_of(&path.to_language_string(), &prefix)
    } else {
        None
    };
    if options.view == View::Hex {
        return hex_records(ctx, fs, &file, compression, selector, &params, options);
    }
    if let Some(algorithm) = compression {
        return decompressed_records(ctx, fs, &file, algorithm, selector, &params, options);
    }
    let kind = Kind::of(&prefix);
    if kind == Kind::Other {
        return Err(binary_error(path));
    }
    check_document(kind, path, selector, options.view)?;
    if options.view == View::Header && matches!(kind, Kind::Image(_)) {
        let etag = file.meta.etag();
        if let Some(records) = conditional(&file, &etag, options) {
            return Ok(records);
        }
        let handle = fs.open_read(path)?;
        let info = image::inspect(BufReader::new(handle)).map_err(|e| image_error(path, &e))?;
        return Ok(vec![image_record(&file, &etag, &info, None, options)]);
    }
    let bytes = read_whole(fs, &file, ctx.limits().max_file_bytes_text)?;
    let etag = hashed_etag(&file, &bytes);
    if let Some(records) = conditional(&file, &etag, options) {
        return Ok(records);
    }
    loaded_document(ctx, &file, kind, bytes, &etag, &params, options)
}

/// File being read.
struct File<'a> {
    path: &'a AbsPath,
    /// Metadata of the entry itself, a link in the last component not
    /// followed, as the journal observes it.
    meta: FileMeta,
    /// Size of the content in bytes.
    size: u64,
}

/// Kind of content recognised from its leading bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pdf,
    Image(&'static str),
    Other,
}

impl Kind {
    fn of(prefix: &[u8]) -> Self {
        if prefix.starts_with(PDF_SIGNATURE) {
            Self::Pdf
        } else if let Some(mime) = image::image_mime(prefix) {
            Self::Image(mime)
        } else {
            Self::Other
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Pdf => "a PDF document",
            Self::Image(_) => "an image",
            Self::Other => "binary content",
        }
    }

    fn selection_hint(self) -> &'static str {
        match self {
            Self::Pdf => "select pages with `pages`",
            Self::Image(_) => "read the whole image",
            Self::Other => "dump its bytes with `view: hex`",
        }
    }
}

/// Extraction of the pages of a PDF document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PdfMode {
    Text(PdfTextMode),
    Images,
}

/// Parameters of `read` that only content that is not text uses.
#[derive(Debug)]
struct Params {
    pages: Option<Vec<Range>>,
    mode: PdfMode,
    decompress: bool,
}

impl Params {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let mode = match choice(args, MODE, &MODES)? {
            Some(MODE_LAYOUT) => PdfMode::Text(PdfTextMode::Layout),
            Some(MODE_IMAGES) => PdfMode::Images,
            _ => PdfMode::Text(PdfTextMode::Text),
        };
        Ok(Self {
            pages: ranges_param(args, PAGES)?,
            mode,
            decompress: choice(args, DECOMPRESS, &DECOMPRESS_VALUES)? != Some(DECOMPRESS_NONE),
        })
    }
}

/// Value of an enumerated parameter; `None` when it is not given.
fn choice<'a>(args: &'a BoundArgs, name: &str, values: &[&'static str]) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(value) | Value::Text(value)) => {
            if values.contains(&value.as_str()) {
                Ok(Some(value.as_str()))
            } else {
                Err(Error::new(
                    ErrorCode::UnknownValue,
                    format!(
                        "`{value}` is not a value of `{name}`: use one of {}",
                        values.join(", ")
                    ),
                )
                .with_found(value.as_str())
                .with_expected(serde_json::json!(values)))
            }
        }
        Some(_) => Err(type_error(name, "a value name")),
    }
}

/// Ranges of a `range | list<range>` parameter counted from 1.
fn ranges_param(args: &BoundArgs, name: &str) -> Result<Option<Vec<Range>>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::List(items)) if items.is_empty() => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` needs at least one range"),
        )),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| one_range(name, item))
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(other) => one_range(name, other).map(|range| Some(vec![range])),
    }
}

fn one_range(name: &str, value: &Value) -> Result<Range> {
    let range = match value {
        Value::Range(range) => *range,
        Value::Int(number) => Range::single(*number),
        Value::Text(text) => parse_range_text(name, text)?,
        other => {
            return Err(type_error(
                name,
                &format!("a range such as `1..5`, not a {}", other.type_name()),
            ));
        }
    };
    for bound in [range.start(), range.end()].into_iter().flatten() {
        if bound < 1 {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{name}` counts from 1, not from {bound}: give numbers of 1 or more"),
            )
            .with_found(range.to_string()));
        }
    }
    Ok(range)
}

fn parse_range_text(name: &str, text: &str) -> Result<Range> {
    let written = text.trim();
    let invalid = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{written}` is not a range of `{name}`: write `2`, `1..5`, `4..` or `..9`"),
        )
        .with_found(written)
    };
    let bound = |part: &str| -> Result<Option<i64>> {
        let part = part.trim();
        if part.is_empty() {
            Ok(None)
        } else {
            part.parse().map(Some).map_err(|_| invalid())
        }
    };
    match written.split_once("..") {
        Some((start, end)) => Range::new(bound(start)?, bound(end)?).map_err(|_| invalid()),
        None => bound(written)?.map(Range::single).ok_or_else(invalid),
    }
}

fn type_error(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{COMMAND}` expects {expected} for `{name}`"),
    )
}

/// Name of the selector; `None` for the whole file.
fn selector_name(selector: &Selector) -> Option<&'static str> {
    match selector {
        Selector::Whole => None,
        Selector::Lines(_) => Some(LINES),
        Selector::Last(_) => Some(LAST),
        Selector::Around(_) => Some(AROUND),
        Selector::Section(_) => Some(SECTION),
        Selector::Symbol(_) => Some(SYMBOL),
        Selector::Key(_) => Some(KEY),
        Selector::Cell(_) => Some(CELL),
    }
}

/// Checks that a PDF document or an image is read whole, with the full view
/// or the header view.
fn check_document(kind: Kind, path: &AbsPath, selector: &Selector, view: View) -> Result<()> {
    let shown = path.to_language_string();
    if !matches!(view, View::Full | View::Header) {
        return Err(Error::new(
            ErrorCode::Unsupported,
            format!(
                "`view: {}` applies to text, and `{shown}` is {}: use `view: full` or `view: header`",
                view.as_str(),
                kind.description()
            ),
        )
        .with_found(view.as_str()));
    }
    if let Some(name) = selector_name(selector) {
        return Err(Error::new(
            ErrorCode::Unsupported,
            format!(
                "`{name}` selects text, and `{shown}` is {}: {}",
                kind.description(),
                kind.selection_hint()
            ),
        )
        .with_found(name));
    }
    Ok(())
}

fn count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn io_error(path: &AbsPath, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
}

/// First `max` bytes of a file, or the whole file when it is shorter.
fn read_up_to(fs: &dyn Fs, path: &AbsPath, max: u64) -> Result<Vec<u8>> {
    let mut handle = fs.open_read(path)?;
    let mut bytes = Vec::new();
    if let Some(mapped) = handle.as_bytes() {
        let end = usize::try_from(max).unwrap_or(usize::MAX).min(mapped.len());
        bytes.extend_from_slice(&mapped[..end]);
    } else {
        handle
            .by_ref()
            .take(max)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error(path, &error))?;
    }
    Ok(bytes)
}

/// Whole content of a file of no more than `limit` bytes.
fn read_whole(fs: &dyn Fs, file: &File<'_>, limit: u64) -> Result<Vec<u8>> {
    if file.size > limit {
        return Err(memory_limit_error(file.path, file.size, limit));
    }
    let bytes = read_up_to(fs, file.path, limit.saturating_add(1))?;
    if count(bytes.len()) > limit {
        return Err(memory_limit_error(file.path, count(bytes.len()), limit));
    }
    Ok(bytes)
}

fn memory_limit_error(path: &AbsPath, size: u64, limit: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "`{path}` has {size} bytes, more than the {limit} bytes read at once: dump rows of it with `view: hex` and `lines`"
        ),
    )
    .with_expected(serde_json::json!(limit))
    .with_actual(serde_json::json!(size))
}

fn not_file_error(path: &AbsPath, kind: FileKind) -> Error {
    let shown = path.to_language_string();
    let what = if kind == FileKind::Directory {
        "a directory"
    } else {
        "a special file"
    };
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{shown}` is {what}, not a file: read files, and list directories with `list`"),
    )
    .with_problem("not_file")
    .with_found(shown)
}

fn binary_error(path: &AbsPath) -> Error {
    let shown = path.to_language_string();
    Error::new(
        ErrorCode::Binary,
        format!("`{shown}` has binary content, not text: read it with `view: hex`"),
    )
    .with_fix(format!("{COMMAND} {shown:?} view: hex"))
    .with_detail("path", serde_json::Value::from(shown))
}

fn pdf_error(path: &AbsPath, error: &PdfError) -> Error {
    let base = Error::new(error.code(), format!("`{path}`: {error}"));
    match error {
        PdfError::PageOutOfRange { page, pages } => base
            .with_actual(serde_json::json!(page))
            .with_expected(serde_json::json!(format!("1..{pages}"))),
        _ => base,
    }
}

fn image_error(path: &AbsPath, error: &ImageError) -> Error {
    Error::new(error.code(), format!("`{path}`: {error}"))
}

fn decompress_error(path: &AbsPath, error: &DecompressError) -> Error {
    let base = Error::new(error.code(), format!("`{path}`: {error}"));
    match error {
        DecompressError::Limit { limit } => base.with_expected(serde_json::json!(limit)),
        _ => base,
    }
}

fn images_unavailable() -> Error {
    Error::new(
        ErrorCode::ExternalMissing,
        "`mode: images` needs the PDFium library, which this build does not include: read the text of the pages with `mode: text` or `mode: layout`",
    )
    .with_found(MODE_IMAGES)
}

fn hashed_etag(file: &File<'_>, bytes: &[u8]) -> Etag {
    Etag {
        hash: Some(content_hash(bytes)),
        ..file.meta.etag()
    }
}

fn base_content(file: &File<'_>, etag: &Etag) -> Content {
    Content {
        path: path_field(file.path),
        size: Size::from_bytes(file.size),
        etag: etag_text(etag),
        encoding: BINARY_ENCODING.to_owned(),
        bom: false,
        line_ending: record_line_ending(LineEnding::None),
        ..Content::default()
    }
}

/// Record of content returned with a `since` that cannot be compared,
/// because only text keeps earlier versions.
fn finish(mut content: Content, options: &ReadOptions) -> Record {
    if options.since.is_some() {
        content.since_missing = Some(true);
    }
    Record::from(content)
}

/// Result of `known` and `unchanged_since` for the version `etag`: no
/// records for a known file, a record without content for an unchanged
/// one, and `None` when the content is to be read.
fn conditional(file: &File<'_>, etag: &Etag, options: &ReadOptions) -> Option<Vec<Record>> {
    if options
        .known
        .iter()
        .any(|(path, tag)| path == file.path && tag.matches(etag))
    {
        return Some(Vec::new());
    }
    options
        .unchanged_since
        .as_ref()
        .filter(|tag| tag.matches(etag))
        .map(|_| {
            vec![Record::from(Content {
                unchanged: Some(true),
                ..base_content(file, etag)
            })]
        })
}

fn mark_decompressed(records: Vec<Record>, path: &AbsPath) -> Vec<Record> {
    records
        .into_iter()
        .map(|mut record| {
            if record.kind == RecordKind::Content {
                record.set(DECOMPRESSED_FROM, Value::Path(path.to_language_string()));
            }
            record
        })
        .collect()
}

/// Records of a PDF document or an image held in memory.
fn loaded_document(
    ctx: &ExecCtx,
    file: &File<'_>,
    kind: Kind,
    bytes: Vec<u8>,
    etag: &Etag,
    params: &Params,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    match kind {
        Kind::Pdf => pdf_records(ctx, file, &bytes, etag, params, options),
        Kind::Image(_) => {
            let info = image::inspect(Cursor::new(bytes.as_slice()))
                .map_err(|error| image_error(file.path, &error))?;
            let data = (options.view != View::Header).then_some(bytes);
            Ok(vec![image_record(file, etag, &info, data, options)])
        }
        Kind::Other => Err(binary_error(file.path)),
    }
}

fn image_record(
    file: &File<'_>,
    etag: &Etag,
    info: &ImageInfo,
    data: Option<Vec<u8>>,
    options: &ReadOptions,
) -> Record {
    let header = options.view == View::Header;
    finish(
        Content {
            binary: Some(true),
            image: Some(ContentImage {
                mime: info.mime().to_owned(),
                width: info.width().map(u64::from),
                height: info.height().map(u64::from),
                data: data.map(Bytes::new),
            }),
            view: header.then(|| View::Header.as_str().to_owned()),
            ..base_content(file, etag)
        },
        options,
    )
}

/// Bounds of a page range for [`PdfDocument::page_range`].
fn page_bounds(range: Range) -> (Bound<u32>, Bound<u32>) {
    let page = |number: i64| u32::try_from(number).unwrap_or(u32::MAX);
    (
        range
            .start()
            .map_or(Bound::Unbounded, |start| Bound::Included(page(start))),
        range
            .end()
            .map_or(Bound::Unbounded, |end| Bound::Included(page(end))),
    )
}

/// One record per page, in the order of the ranges of `pages`.
fn pdf_records(
    ctx: &ExecCtx,
    file: &File<'_>,
    bytes: &[u8],
    etag: &Etag,
    params: &Params,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let document = PdfDocument::parse(bytes).map_err(|error| pdf_error(file.path, &error))?;
    let total = u64::from(document.page_count());
    let base = || Content {
        pages_total: Some(total),
        ..base_content(file, etag)
    };
    if options.view == View::Header {
        let header = Content {
            view: Some(View::Header.as_str().to_owned()),
            ..base()
        };
        return Ok(vec![finish(header, options)]);
    }
    let PdfMode::Text(mode) = params.mode else {
        return Err(images_unavailable());
    };
    let ranges = match &params.pages {
        Some(ranges) => ranges.clone(),
        None if total == 0 => return Ok(vec![finish(base(), options)]),
        None => vec![Range::full()],
    };
    let mut records = Vec::new();
    for range in ranges {
        let pages = document
            .page_range(page_bounds(range))
            .map_err(|error| pdf_error(file.path, &error))?;
        for number in pages {
            ctx.checkpoint()?;
            let text = document
                .page_text(number, mode)
                .map_err(|error| pdf_error(file.path, &error))?;
            let page = Content {
                lines: count(text.lines().count()),
                text,
                page: Some(u64::from(number)),
                ..base()
            };
            records.push(finish(page, options));
        }
    }
    Ok(records)
}

/// Encoding of decompressed content: the given one, or the detected one;
/// `None` for content that is not text.
fn text_encoding(ctx: &ExecCtx, bytes: &[u8], encoding: EncodingParam) -> Option<TextEncoding> {
    match encoding {
        EncodingParam::Auto => text_codec(ctx).detect(bytes).encoding,
        EncodingParam::Codec(codec) => {
            Some(codec.text_encoding(codec.bom().is_some_and(|bom| bytes.starts_with(bom))))
        }
    }
}

/// Records of a compressed file read after decompression.
fn decompressed_records(
    ctx: &ExecCtx,
    fs: &dyn Fs,
    file: &File<'_>,
    algorithm: Algorithm,
    selector: &Selector,
    params: &Params,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let limit = ctx.limits().max_file_bytes_text;
    let raw = read_whole(fs, file, limit)?;
    let etag = hashed_etag(file, &raw);
    if let Some(records) = conditional(file, &etag, options) {
        return Ok(mark_decompressed(records, file.path));
    }
    let bytes = compressed::read_limited(algorithm, raw.as_slice(), limit)
        .map_err(|error| decompress_error(file.path, &error))?;
    ctx.note(at_path(Notable::Decompressed, file.path));
    let records = match text_encoding(ctx, &bytes, options.encoding) {
        Some(encoding) => decompressed_text(ctx, file, &raw, &bytes, encoding, selector, options)?,
        None => {
            let kind = Kind::of(&bytes);
            if kind == Kind::Other {
                return Err(binary_error(file.path));
            }
            check_document(kind, file.path, selector, options.view)?;
            loaded_document(ctx, file, kind, bytes, &etag, params, options)?
        }
    };
    Ok(mark_decompressed(records, file.path))
}

/// Records of decompressed text, read as a text file with the version of
/// the compressed file.
///
/// The text is kept in the read cache registered by the engine, so that a
/// later `since` finds it; without one, `since` reports `since_missing`.
fn decompressed_text(
    ctx: &ExecCtx,
    file: &File<'_>,
    raw: &[u8],
    bytes: &[u8],
    encoding: TextEncoding,
    selector: &Selector,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let shown = file.path.to_language_string();
    let text = decode(bytes, &encoding).map_err(|error| error.into_error(Some(&shown)))?;
    let source = TextSource {
        path: file.path.clone(),
        meta: file.meta.clone(),
        size: file.size,
        line_ending: line_ending(bytes, &encoding),
        hash: content_hash(raw),
        text,
        encoding,
    };
    let shared = ctx.extensions().get::<dyn ReadCache>();
    let local;
    let cache: &dyn ReadCache = match &shared {
        Some(cache) => cache.as_ref(),
        None => {
            local = MemoryReadCache::default();
            &local
        }
    };
    text::read_source(ctx, cache, &source, selector, options)
}

/// Rows of a dump to produce.
#[derive(Debug, Clone)]
enum Rows {
    All,
    Ranges(Vec<Range>),
    Last(u64),
}

impl Rows {
    fn select(selector: &Selector, pages: Option<&Vec<Range>>) -> Result<Self> {
        match (selector, pages) {
            (Selector::Whole, None) => Ok(Self::All),
            (Selector::Whole, Some(pages)) => Ok(Self::Ranges(pages.clone())),
            (Selector::Lines(ranges), None) => Ok(Self::Ranges(ranges.clone())),
            (Selector::Last(rows), None) => Ok(Self::Last(*rows)),
            (Selector::Lines(_) | Selector::Last(_), Some(_)) => Err(Error::new(
                ErrorCode::ParamConflict,
                format!(
                    "`view: hex` takes its rows from one of `{LINES}`, `{LAST}` and `{PAGES}`: remove the others"
                ),
            )),
            (other, _) => {
                let name = selector_name(other).unwrap_or_default();
                Err(Error::new(
                    ErrorCode::Unsupported,
                    format!(
                        "`{name}` selects text and does not apply to `view: hex`: select rows with `{LINES}`, `{LAST}` or `{PAGES}`"
                    ),
                )
                .with_found(name))
            }
        }
    }
}

/// Rows of one requested range, numbered from 1.
#[derive(Debug, Default)]
struct Part {
    first: Option<u64>,
    last: u64,
    text: String,
}

/// Budget of text of one dump.
struct Budget<'a> {
    path: &'a AbsPath,
    limit: u64,
    used: u64,
}

impl Budget<'_> {
    fn append(&mut self, part: &mut Part, number: u64, row: &HexRow) -> Result<()> {
        let line = row.to_string();
        self.used = self.used.saturating_add(count(line.len()) + 1);
        if self.used > self.limit {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "the dump of `{}` exceeds the {} bytes of text of one record: select rows with `{LINES}` or `{LAST}`",
                    self.path, self.limit
                ),
            )
            .with_expected(serde_json::json!(self.limit)));
        }
        part.text.push_str(&line);
        part.text.push('\n');
        part.first.get_or_insert(number);
        part.last = number;
        Ok(())
    }
}

/// Row number of an inclusive bound of a range, from 1.
fn row_bound(bound: Option<i64>) -> Option<u64> {
    bound.map(|number| u64::try_from(number).unwrap_or(0))
}

fn row_range(first: u64, last: u64) -> Range {
    let row = |number: u64| i64::try_from(number).unwrap_or(i64::MAX);
    Range::new(Some(row(first)), Some(row(last))).unwrap_or_else(|_| Range::single(row(first)))
}

/// Dump of seekable content of `size` bytes; rows outside the selection are
/// skipped without reading them.
fn seekable_dump<R: io::Read + Seek + ?Sized>(
    reader: &mut R,
    size: u64,
    rows: &Rows,
    budget: &mut Budget<'_>,
) -> Result<(Vec<Part>, Option<u64>)> {
    let total = hex::row_count(size);
    let spans: Vec<(u64, u64)> = match rows {
        Rows::All => vec![(1, total)],
        Rows::Last(last) => vec![(total.saturating_sub(*last) + 1, total)],
        Rows::Ranges(ranges) => ranges
            .iter()
            .map(|range| {
                (
                    row_bound(range.start()).unwrap_or(1),
                    row_bound(range.end()).map_or(total, |end| end.min(total)),
                )
            })
            .collect(),
    };
    let mut parts = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        let mut part = Part::default();
        if first <= last {
            let dump = HexRows::seek_to_row(&mut *reader, first - 1)
                .map_err(|error| io_error(budget.path, &error))?;
            for (number, row) in (first..=last).zip(dump) {
                let row = row.map_err(|error| io_error(budget.path, &error))?;
                budget.append(&mut part, number, &row)?;
            }
        }
        parts.push(part);
    }
    Ok((parts, Some(total)))
}

/// Dump of a stream read once from its start; the number of rows is known
/// when the stream was read to its end.
fn streamed_dump<R: io::Read>(
    reader: R,
    rows: &Rows,
    budget: &mut Budget<'_>,
    map_error: impl Fn(io::Error) -> Error,
) -> Result<(Vec<Part>, Option<u64>)> {
    let dump = HexRows::new(reader);
    let mut number = 0u64;
    match rows {
        Rows::All => {
            let mut part = Part::default();
            for row in dump {
                number += 1;
                budget.append(&mut part, number, &row.map_err(&map_error)?)?;
            }
            Ok((vec![part], Some(number)))
        }
        Rows::Last(last) => {
            let keep = usize::try_from(*last).unwrap_or(usize::MAX);
            let mut kept: VecDeque<HexRow> = VecDeque::new();
            for row in dump {
                number += 1;
                if kept.len() == keep {
                    kept.pop_front();
                }
                kept.push_back(row.map_err(&map_error)?);
            }
            let mut part = Part::default();
            let first = number + 1 - count(kept.len());
            for (row_number, row) in (first..).zip(&kept) {
                budget.append(&mut part, row_number, row)?;
            }
            Ok((vec![part], Some(number)))
        }
        Rows::Ranges(ranges) => {
            let end = ranges
                .iter()
                .map(|range| row_bound(range.end()))
                .collect::<Option<Vec<u64>>>()
                .and_then(|ends| ends.into_iter().max());
            let mut parts: Vec<Part> = ranges.iter().map(|_| Part::default()).collect();
            let mut complete = true;
            for row in dump {
                number += 1;
                if end.is_some_and(|end| number > end) {
                    complete = false;
                    break;
                }
                let row = row.map_err(&map_error)?;
                let position = i64::try_from(number).unwrap_or(i64::MAX);
                for (part, range) in parts.iter_mut().zip(ranges) {
                    if range.contains(position) {
                        budget.append(part, number, &row)?;
                    }
                }
            }
            Ok((parts, complete.then_some(number)))
        }
    }
}

/// Record of a hexadecimal dump of a file, after decompression when
/// `compression` is set.
fn hex_records(
    ctx: &ExecCtx,
    fs: &dyn Fs,
    file: &File<'_>,
    compression: Option<Algorithm>,
    selector: &Selector,
    params: &Params,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let rows = Rows::select(selector, params.pages.as_ref())?;
    let etag = file.meta.etag();
    let mark = |records: Vec<Record>| match compression {
        Some(_) => mark_decompressed(records, file.path),
        None => records,
    };
    if let Some(records) = conditional(file, &etag, options) {
        return Ok(mark(records));
    }
    let limit = ctx.limits().max_file_bytes_text;
    let mut budget = Budget {
        path: file.path,
        limit,
        used: 0,
    };
    let mut handle = fs.open_read(file.path)?;
    let (parts, total) = match compression {
        None => seekable_dump(&mut *handle, file.size, &rows, &mut budget)?,
        Some(algorithm) => {
            let reader = DecompressedReader::new(algorithm, handle, limit)
                .map_err(|error| decompress_error(file.path, &error))?;
            ctx.note(at_path(Notable::Decompressed, file.path));
            streamed_dump(reader, &rows, &mut budget, |error| {
                decompress_error(file.path, &DecompressError::from_io(error))
            })?
        }
    };
    let mut text = String::new();
    let mut lines = 0;
    let mut spans = Vec::new();
    for part in parts {
        if let Some(first) = part.first {
            spans.push(row_range(first, part.last));
            lines += part.last - first + 1;
            text.push_str(&part.text);
        }
    }
    let (range, ranges) = if matches!(rows, Rows::All) {
        (None, None)
    } else if let &[single] = spans.as_slice() {
        (Some(single), None)
    } else {
        (None, Some(spans))
    };
    let content = Content {
        text,
        lines,
        total_lines: total,
        range,
        ranges,
        view: Some(View::Hex.as_str().to_owned()),
        ..base_content(file, &etag)
    };
    Ok(mark(vec![finish(content, options)]))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use ::image::{DynamicImage, ImageFormat};
    use nurt_archive::codec;
    use nurt_contracts::stage::{ArgValue, Limits, ReadStage, StageData, StageInput};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    use super::super::{Read, VIEW};
    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn file(test: &TestContext, path: &str, content: &[u8]) -> AbsPath {
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        path
    }

    fn args(path: &AbsPath, pairs: Vec<(&str, Value)>) -> BoundArgs {
        pairs.into_iter().fold(
            BoundArgs::new(COMMAND).with(
                "path",
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            ),
            |args, (name, value)| args.with(name, ArgValue::Value(value), false),
        )
    }

    fn run(ctx: &ExecCtx, path: &AbsPath, pairs: Vec<(&str, Value)>) -> Result<Vec<Content>> {
        let stage = Read::default();
        let args = args(path, pairs);
        let output = stage.execute(&args, StageInput::none(), ctx)?;
        match output.data {
            StageData::Records(stream) => stream
                .map(|record| record.map(|record| Content::try_from(record).unwrap()))
                .collect(),
            _ => panic!("records expected"),
        }
    }

    fn hex() -> (&'static str, Value) {
        (VIEW, Value::Enum("hex".to_owned()))
    }

    fn range(first: i64, last: i64) -> Range {
        Range::new(Some(first), Some(last)).unwrap()
    }

    fn compress(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut encoder = codec::encoder(algorithm, None, Vec::new()).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn limited(test: &TestContext, limit: u64) -> ExecCtx {
        test.builder()
            .limits(Limits {
                max_file_bytes_text: limit,
                ..Limits::default()
            })
            .build()
    }

    /// 64 bytes, four rows of the dump.
    fn sixty_four() -> Vec<u8> {
        (0u8..64).collect()
    }

    #[test]
    fn whole_dump_matches_xxd_and_counts_rows() {
        let test = TestContext::new();
        let path = file(&test, "/p/a.bin", b"#include <stdio.h>");
        let read = run(&test.ctx(), &path, vec![hex()]).unwrap();
        assert_eq!(
            read[0].text,
            "00000000: 2369 6e63 6c75 6465 203c 7374 6469 6f2e  #include <stdio.\n00000010: 683e                                     h>\n"
        );
        assert_eq!(read[0].lines, 2);
        assert_eq!(read[0].total_lines, Some(2));
        assert_eq!(read[0].view.as_deref(), Some("hex"));
        assert_eq!(read[0].encoding, BINARY_ENCODING);
        assert_eq!((read[0].range, read[0].ranges.clone()), (None, None));
        assert!(!read[0].etag.contains("sha256:"));
    }

    #[test]
    fn rows_are_selected_with_lines_last_and_pages() {
        let test = TestContext::new();
        let path = file(&test, "/p/a.bin", &sixty_four());
        let ctx = test.ctx();
        let several = run(
            &ctx,
            &path,
            vec![
                hex(),
                (
                    LINES,
                    Value::List(vec![Value::Range(range(3, 4)), Value::Int(1)]),
                ),
            ],
        )
        .unwrap();
        assert_eq!(several[0].ranges, Some(vec![range(3, 4), range(1, 1)]));
        assert_eq!(several[0].lines, 3);
        assert!(several[0].text.starts_with("00000020: 2021 "));
        assert!(
            several[0].text.ends_with(
                "\n00000000: 0001 0203 0405 0607 0809 0a0b 0c0d 0e0f  ................\n"
            )
        );
        let last = run(&ctx, &path, vec![hex(), (LAST, Value::Int(1))]).unwrap();
        assert_eq!(last[0].range, Some(range(4, 4)));
        assert!(last[0].text.starts_with("00000030: "));
        let pages = run(
            &ctx,
            &path,
            vec![hex(), (PAGES, Value::Text("2..3".to_owned()))],
        )
        .unwrap();
        assert_eq!(pages[0].range, Some(range(2, 3)));
        let beyond = run(
            &ctx,
            &path,
            vec![hex(), (LINES, Value::Range(range(9, 10)))],
        )
        .unwrap();
        assert_eq!(beyond[0].text, "");
        assert_eq!(beyond[0].ranges, Some(Vec::new()));
        assert_eq!(beyond[0].total_lines, Some(4));
    }

    #[test]
    fn dump_rows_are_selected_once_and_only_by_row_selectors() {
        let test = TestContext::new();
        let path = file(&test, "/p/a.bin", &sixty_four());
        let ctx = test.ctx();
        let twice = run(
            &ctx,
            &path,
            vec![hex(), (LINES, Value::Int(1)), (PAGES, Value::Int(2))],
        )
        .unwrap_err();
        assert_eq!(twice.code(), ErrorCode::ParamConflict);
        let around = run(
            &ctx,
            &path,
            vec![hex(), (AROUND, Value::Text("x".to_owned()))],
        )
        .unwrap_err();
        assert_eq!(around.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn dump_above_the_text_limit_fails() {
        let test = TestContext::new();
        let path = file(&test, "/p/a.bin", &sixty_four());
        let ctx = limited(&test, 100);
        assert_eq!(
            run(&ctx, &path, vec![hex()]).unwrap_err().code(),
            ErrorCode::Limit
        );
        assert_eq!(
            run(&ctx, &path, vec![hex(), (LINES, Value::Int(1))]).unwrap()[0].lines,
            1
        );
    }

    #[test]
    fn dump_of_a_directory_is_invalid() {
        let test = TestContext::new();
        file(&test, "/p/d/x.bin", b"x");
        let error = run(&test.ctx(), &abs("/p/d"), vec![hex()]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn compressed_text_is_read_after_decompression() {
        let test = TestContext::new();
        let path = file(
            &test,
            "/l/app.log.gz",
            &compress(Algorithm::Gzip, b"start\nstop\n"),
        );
        let ctx = test.ctx();
        let read = run(&ctx, &path, vec![(LAST, Value::Int(1))]).unwrap();
        assert_eq!(read[0].text, "stop\n");
        assert_eq!(read[0].encoding, "utf-8");
        assert_eq!(
            read[0].decompressed_from.as_ref().map(|from| from.as_str()),
            Some(path.to_language_string().as_str())
        );
        assert!(read[0].etag.contains("sha256:"));
        let notes = ctx.take_notes();
        assert!(notes.iter().any(|note| note.code == Notable::Decompressed));
    }

    #[test]
    fn every_automatic_format_is_decompressed() {
        let test = TestContext::new();
        let ctx = test.ctx();
        for (algorithm, name) in [
            (Algorithm::Zstd, "/l/a.log.zst"),
            (Algorithm::Xz, "/l/a.log.xz"),
            (Algorithm::Bzip2, "/l/a.log.bz2"),
        ] {
            let path = file(&test, name, &compress(algorithm, b"one\ntwo\n"));
            assert_eq!(
                run(&ctx, &path, Vec::new()).unwrap()[0].text,
                "one\ntwo\n",
                "{name}"
            );
        }
    }

    #[test]
    fn decompress_none_keeps_compressed_content_binary() {
        let test = TestContext::new();
        let path = file(
            &test,
            "/l/app.log.gz",
            &compress(Algorithm::Gzip, b"start\n"),
        );
        let ctx = test.ctx();
        let none = (DECOMPRESS, Value::Enum(DECOMPRESS_NONE.to_owned()));
        assert_eq!(
            run(&ctx, &path, vec![none.clone()]).unwrap_err().code(),
            ErrorCode::Binary
        );
        let raw = run(&ctx, &path, vec![hex(), none]).unwrap();
        assert!(raw[0].text.starts_with("00000000: 1f8b "));
        assert_eq!(raw[0].decompressed_from, None);
    }

    #[test]
    fn dump_of_a_compressed_file_shows_decompressed_bytes() {
        let test = TestContext::new();
        let path = file(&test, "/l/a.gz", &compress(Algorithm::Gzip, b"abc"));
        let read = run(&test.ctx(), &path, vec![hex()]).unwrap();
        assert_eq!(
            read[0].text,
            format!("00000000: 6162 63{}abc\n", " ".repeat(34))
        );
        assert_eq!(read[0].total_lines, Some(1));
        assert!(read[0].decompressed_from.is_some());
    }

    #[test]
    fn streamed_dump_selects_ranges_and_last_rows() {
        let test = TestContext::new();
        let path = file(&test, "/l/a.gz", &compress(Algorithm::Gzip, &sixty_four()));
        let ctx = test.ctx();
        let ranges = run(
            &ctx,
            &path,
            vec![
                hex(),
                (
                    LINES,
                    Value::List(vec![Value::Range(range(2, 2)), Value::Int(1)]),
                ),
            ],
        )
        .unwrap();
        assert_eq!(ranges[0].ranges, Some(vec![range(2, 2), range(1, 1)]));
        assert!(ranges[0].text.starts_with("00000010: "));
        assert_eq!(ranges[0].total_lines, None);
        let open = run(
            &ctx,
            &path,
            vec![hex(), (LINES, Value::Range(Range::from_start(3)))],
        )
        .unwrap();
        assert_eq!(open[0].range, Some(range(3, 4)));
        assert_eq!(open[0].total_lines, Some(4));
        let last = run(&ctx, &path, vec![hex(), (LAST, Value::Int(2))]).unwrap();
        assert_eq!(last[0].range, Some(range(3, 4)));
    }

    #[test]
    fn decompression_beyond_the_limit_fails() {
        let test = TestContext::new();
        let path = file(
            &test,
            "/l/bomb.gz",
            &compress(Algorithm::Gzip, &vec![0; 100_000]),
        );
        let error = run(&limited(&test, 1000), &path, Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
        assert!(
            error.message.contains("decompress: none"),
            "{}",
            error.message
        );
    }

    #[test]
    fn corrupt_compressed_data_is_a_format_error() {
        let test = TestContext::new();
        let mut damaged = b"\x1f\x8b\x08\x00\x00\x00\x00\x00".to_vec();
        damaged.extend_from_slice(&[0x55; 40]);
        let path = file(&test, "/l/a.gz", &damaged);
        let error = run(&test.ctx(), &path, Vec::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
    }

    #[test]
    fn decompressed_binary_content_of_no_known_kind_is_binary() {
        let test = TestContext::new();
        let path = file(
            &test,
            "/l/a.bin.gz",
            &compress(Algorithm::Gzip, &[0, 1, 2, 3, 0, 0]),
        );
        assert_eq!(
            run(&test.ctx(), &path, Vec::new()).unwrap_err().code(),
            ErrorCode::Binary
        );
    }

    #[test]
    fn images_carry_type_dimensions_and_data() {
        let test = TestContext::new();
        let bytes = png(3, 2);
        let path = file(&test, "/i/logo.png", &bytes);
        let ctx = test.ctx();
        let full = run(&ctx, &path, Vec::new()).unwrap();
        let image = full[0].image.clone().unwrap();
        assert_eq!(image.mime, "image/png");
        assert_eq!((image.width, image.height), (Some(3), Some(2)));
        assert_eq!(image.data.map(|data| data.as_slice().to_vec()), Some(bytes));
        assert_eq!(full[0].binary, Some(true));
        assert_eq!(full[0].text, "");
        let header = run(&ctx, &path, vec![(VIEW, Value::Enum("header".to_owned()))]).unwrap();
        let image = header[0].image.clone().unwrap();
        assert_eq!(image.data, None);
        assert_eq!(image.width, Some(3));
        assert_eq!(header[0].view.as_deref(), Some("header"));
        assert!(!header[0].etag.contains("sha256:"));
    }

    #[test]
    fn compressed_images_are_read_after_decompression() {
        let test = TestContext::new();
        let path = file(
            &test,
            "/i/logo.png.zst",
            &compress(Algorithm::Zstd, &png(4, 4)),
        );
        let read = run(&test.ctx(), &path, Vec::new()).unwrap();
        assert_eq!(
            read[0].image.as_ref().map(|image| image.width),
            Some(Some(4))
        );
        assert!(read[0].decompressed_from.is_some());
    }

    #[test]
    fn text_selectors_and_views_do_not_apply_to_images() {
        let test = TestContext::new();
        let path = file(&test, "/i/logo.png", &png(2, 2));
        let ctx = test.ctx();
        let lines = run(&ctx, &path, vec![(LINES, Value::Int(1))]).unwrap_err();
        assert_eq!(lines.code(), ErrorCode::Unsupported);
        let outline =
            run(&ctx, &path, vec![(VIEW, Value::Enum("outline".to_owned()))]).unwrap_err();
        assert_eq!(outline.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn conditional_reads_apply_to_images() {
        let test = TestContext::new();
        let path = file(&test, "/i/logo.png", &png(2, 2));
        let ctx = test.ctx();
        let first = run(&ctx, &path, Vec::new()).unwrap();
        let etag = Value::Text(first[0].etag.clone());
        let mut known = nurt_contracts::value::Fields::new();
        known.insert(path.to_language_string(), etag.clone());
        assert!(
            run(&ctx, &path, vec![("known", Value::Map(known))])
                .unwrap()
                .is_empty()
        );
        let unchanged = run(&ctx, &path, vec![("unchanged_since", etag.clone())]).unwrap();
        assert_eq!(unchanged[0].unchanged, Some(true));
        assert_eq!(unchanged[0].image, None);
        let since = run(&ctx, &path, vec![("since", etag)]).unwrap();
        assert_eq!(since[0].since_missing, Some(true));
        assert!(since[0].image.is_some());
    }

    #[test]
    fn parameter_values_of_binary_reads_are_checked() {
        let test = TestContext::new();
        let path = file(&test, "/i/logo.png", &png(2, 2));
        let ctx = test.ctx();
        let code =
            |name: &str, value: Value| run(&ctx, &path, vec![(name, value)]).unwrap_err().code();
        assert_eq!(
            code(MODE, Value::Enum("pages".to_owned())),
            ErrorCode::UnknownValue
        );
        assert_eq!(
            code(DECOMPRESS, Value::Text("zip".to_owned())),
            ErrorCode::UnknownValue
        );
        assert_eq!(code(MODE, Value::Int(1)), ErrorCode::Type);
        assert_eq!(code(PAGES, Value::Int(0)), ErrorCode::InvalidValue);
        assert_eq!(code(PAGES, Value::Bool(true)), ErrorCode::Type);
        assert_eq!(
            code(PAGES, Value::List(Vec::new())),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            code(PAGES, Value::Text("two".to_owned())),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn range_text_accepts_open_and_single_bounds() {
        assert_eq!(
            parse_range_text(PAGES, "4..").unwrap(),
            Range::from_start(4)
        );
        assert_eq!(parse_range_text(PAGES, " ..9 ").unwrap(), Range::up_to(9));
        assert_eq!(parse_range_text(PAGES, "2").unwrap(), Range::single(2));
        assert!(parse_range_text(PAGES, "3..1").is_err());
    }

    #[test]
    fn kinds_are_recognised_from_leading_bytes() {
        assert_eq!(Kind::of(b"%PDF-1.7\n"), Kind::Pdf);
        assert_eq!(Kind::of(&png(1, 1)), Kind::Image("image/png"));
        assert_eq!(Kind::of(b"\x7fELF"), Kind::Other);
        assert_eq!(
            Kind::Other.selection_hint(),
            "dump its bytes with `view: hex`"
        );
    }

    #[test]
    fn images_mode_reports_the_missing_library() {
        let error = images_unavailable();
        assert_eq!(error.code(), ErrorCode::ExternalMissing);
        assert!(error.message.contains("mode: text"));
    }
}
