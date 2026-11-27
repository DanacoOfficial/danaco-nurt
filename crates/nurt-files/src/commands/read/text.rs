// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::read::text
// Responsibility:   Text content of files for `read`: loading and decoding, `content`.
// Dependencies:     nurt_contracts, nurt_notebook, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Text content of files for `read`: loading and decoding, `content`
//! records of selected lines, conditional reads, and the outlines, symbols,
//! document keys and notebook cells served by other command groups.
//!
//! The entity tag of a `content` record carries the SHA-256 hash of the file
//! bytes. `unchanged_since`, `since` and `known` accept such a tag, or the
//! content hash alone, or its first hexadecimal digits (at least
//! [`MIN_HASH_PREFIX`]).

use std::io::Read as _;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag, FileKind, FileMeta, Follow, Fs};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::record::{self, Cell, Content, Hunk, Item, Symbol, SymbolSource, ValueType};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{
    self, DocItem, LanguageId, LineEnding, SymbolInfo, TextCodecService, TextEncoding,
};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{self, Record, Size, Value};
use nurt_notebook::{CellData, CellRef};
use nurt_text::codec::decode;
use nurt_text::codec_service::line_ending;
use nurt_text::diff::{self, unified_diff};
use nurt_text::hash::content_hash;

use super::cache::{ReadCache, hash_digits};
use super::select::{self, LineSpan, Lines, Selector};
use crate::common::etag::{etag_text, parse_etag};
use crate::common::notable::at_path;
use crate::common::params::EncodingParam;
use crate::common::path::path_field;
use crate::common::text::{record_line_ending, text_codec};

/// Fewest hexadecimal digits of a content hash accepted in place of an
/// entity tag.
pub(crate) const MIN_HASH_PREFIX: usize = 8;

/// Hexadecimal digits of a SHA-256 hash.
const HASH_DIGITS: usize = 64;

/// Bytes examined to tell text from binary content in a file larger than
/// the text limit.
const DETECTION_PREFIX: u64 = 64 * 1024;

/// Lines of context of the hunks of `since`, as in `diff -u`.
const DIFF_CONTEXT: usize = 3;

/// Separator between a line number and the line with `line_numbers`.
const NUMBER_SEPARATOR: &str = "  ";

/// Indentation of one nesting level in the text of `view: outline`.
const OUTLINE_INDENT: &str = "  ";

/// File name extension of notebooks.
const NOTEBOOK_EXTENSION: &str = "ipynb";

const NO_CODE_SERVICE: &str = "code analysis is not available; the full content is returned";
const NO_GRAMMAR: &str = "no grammar recognises the file; the full content is returned";
const NO_DOC_SERVICE: &str = "document analysis is not available; the full content is returned";

/// View of the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    /// The text.
    Full,
    /// The symbols of the file as indented text.
    Outline,
    /// The symbols of the file followed by the text.
    OutlineFirst,
    /// The symbols of the file without text.
    Symbols,
    /// Metadata of the file without text.
    Header,
    /// Hexadecimal dump.
    Hex,
}

impl View {
    /// Names of the views, in the order of the catalog.
    pub(crate) const NAMES: [&'static str; 6] = [
        "full",
        "outline",
        "outline_first",
        "symbols",
        "header",
        "hex",
    ];

    /// View named `name`.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "full" => Some(Self::Full),
            "outline" => Some(Self::Outline),
            "outline_first" => Some(Self::OutlineFirst),
            "symbols" => Some(Self::Symbols),
            "header" => Some(Self::Header),
            "hex" => Some(Self::Hex),
            _ => None,
        }
    }

    /// Name of the view.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Outline => "outline",
            Self::OutlineFirst => "outline_first",
            Self::Symbols => "symbols",
            Self::Header => "header",
            Self::Hex => "hex",
        }
    }

    /// Returns `true` when the view can be combined with the selector:
    /// `outline`, `symbols` and `header` describe the whole file, and
    /// `outline_first` takes the selectors of lines.
    pub(crate) fn combines_with(self, selector: &Selector) -> bool {
        match self {
            Self::Full | Self::Hex => true,
            Self::OutlineFirst => matches!(selector, Selector::Whole) || selector.selects_lines(),
            Self::Outline | Self::Symbols | Self::Header => matches!(selector, Selector::Whole),
        }
    }
}

/// Entity tag given to `unchanged_since`, `since` or `known`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TagMatch {
    /// A whole entity tag; its content hash is compared when it has one.
    Etag(Etag),
    /// Leading hexadecimal digits of a content hash, in lower case.
    Hash(String),
}

impl TagMatch {
    /// Reads an entity tag, a content hash or the first digits of one.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for any other text.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let written = text.trim();
        if let Ok(etag) = parse_etag(written) {
            return Ok(Self::Etag(etag));
        }
        let digits = written.strip_prefix(ContentHash::PREFIX).unwrap_or(written);
        if (MIN_HASH_PREFIX..=HASH_DIGITS).contains(&digits.len())
            && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Ok(Self::Hash(digits.to_ascii_lowercase()));
        }
        Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "`{written}` is not an etag: pass the `etag` field of a record read earlier or at least {MIN_HASH_PREFIX} digits of its content hash"
            ),
        )
        .with_problem("etag_syntax")
        .with_found(written))
    }

    /// Returns `true` when the file with the tag `current`, which carries
    /// the content hash, is the version named by this tag.
    pub(crate) fn matches(&self, current: &Etag) -> bool {
        match self {
            Self::Etag(etag) => etag.matches(current),
            Self::Hash(prefix) => current
                .hash
                .as_ref()
                .is_some_and(|hash| hash_digits(hash).starts_with(prefix.as_str())),
        }
    }

    fn hash_prefix(&self) -> Option<String> {
        match self {
            Self::Etag(etag) => etag.hash.as_ref().map(hash_digits),
            Self::Hash(prefix) => Some(prefix.clone()),
        }
    }
}

/// Settings of a `read` call shared by all its targets.
#[derive(Debug, Clone)]
pub(crate) struct ReadOptions {
    /// View.
    pub(crate) view: View,
    /// Lines of context around the matches of `around`.
    pub(crate) context: u64,
    /// Depth of the outline and of the values read at a key.
    pub(crate) depth: Option<u32>,
    /// Encoding.
    pub(crate) encoding: EncodingParam,
    /// Prefix lines with their numbers.
    pub(crate) line_numbers: bool,
    /// Longest line returned, in characters.
    pub(crate) max_line: Option<usize>,
    /// Return no text when the file still has this tag.
    pub(crate) unchanged_since: Option<TagMatch>,
    /// Return the difference since this version.
    pub(crate) since: Option<TagMatch>,
    /// Files skipped while they still have these tags.
    pub(crate) known: Vec<(AbsPath, TagMatch)>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            view: View::Full,
            context: select::DEFAULT_CONTEXT,
            depth: None,
            encoding: EncodingParam::Auto,
            line_numbers: false,
            max_line: None,
            unchanged_since: None,
            since: None,
            known: Vec::new(),
        }
    }
}

/// Decoded text of a file.
#[derive(Debug, Clone)]
pub(crate) struct TextSource {
    /// File.
    pub(crate) path: AbsPath,
    /// Metadata of the entry itself, a link in the last component not
    /// followed, as the journal observes it.
    pub(crate) meta: FileMeta,
    /// Size of the content in bytes.
    pub(crate) size: u64,
    /// Text without the byte order mark.
    pub(crate) text: String,
    /// Encoding.
    pub(crate) encoding: TextEncoding,
    /// Line endings.
    pub(crate) line_ending: LineEnding,
    /// SHA-256 hash of the file bytes.
    pub(crate) hash: ContentHash,
}

impl TextSource {
    /// Entity tag of the file with its content hash.
    pub(crate) fn etag(&self) -> Etag {
        Etag {
            hash: Some(self.hash),
            ..self.meta.etag()
        }
    }
}

/// Content of a file as loaded.
#[derive(Debug)]
pub(crate) enum Loaded {
    /// Text.
    Text(Box<TextSource>),
    /// Content that is not text in any supported encoding.
    Binary,
}

/// Loads a file as text.
///
/// With [`EncodingParam::Auto`] the encoding is detected and binary content
/// gives [`Loaded::Binary`]; with a named encoding the file is decoded in it.
///
/// # Errors
///
/// Fails with the errors of the file system (`E_NOT_FOUND`,
/// `E_PERMISSION`, `E_IO`), with `E_INVALID_VALUE` for a directory or a
/// special file, with `E_LIMIT` for text above the limit of the context
/// and with `E_ENCODING` when the content is not valid in the encoding.
pub(crate) fn load(ctx: &ExecCtx, path: &AbsPath, encoding: EncodingParam) -> Result<Loaded> {
    let fs = ctx.require_fs()?;
    let followed = fs.metadata(path, Follow::Follow)?;
    if followed.kind != FileKind::File {
        return Err(not_file_error(path, followed.kind));
    }
    let meta = fs.metadata(path, Follow::NoFollow)?;
    let limit = ctx.limits().max_file_bytes_text;
    let (bytes, complete) = read_bounded(fs, path, limit)?;
    let detected = match encoding {
        EncodingParam::Auto => match text_codec(ctx).detect(&bytes).encoding {
            Some(detected) => detected,
            None => return Ok(Loaded::Binary),
        },
        EncodingParam::Codec(codec) => {
            codec.text_encoding(codec.bom().is_some_and(|bom| bytes.starts_with(bom)))
        }
    };
    if !complete {
        return Err(limit_error(path, followed.size, limit));
    }
    let text = decode(&bytes, &detected)
        .map_err(|error| error.into_error(Some(&path.to_language_string())))?;
    Ok(Loaded::Text(Box::new(TextSource {
        path: path.clone(),
        meta,
        size: count(bytes.len()),
        line_ending: line_ending(&bytes, &detected),
        hash: content_hash(&bytes),
        text,
        encoding: detected,
    })))
}

/// Reads the whole file when it is within `limit`, otherwise the first
/// [`DETECTION_PREFIX`] bytes; the flag tells whether the content is whole.
fn read_bounded(fs: &dyn Fs, path: &AbsPath, limit: u64) -> Result<(Vec<u8>, bool)> {
    let mut handle = fs.open_read(path)?;
    let size = handle.meta()?.size;
    let within = size <= limit;
    let wanted = if within {
        limit.saturating_add(1)
    } else {
        DETECTION_PREFIX
    };
    let mut bytes = Vec::new();
    if let Some(mapped) = handle.as_bytes() {
        let end = usize::try_from(wanted)
            .unwrap_or(usize::MAX)
            .min(mapped.len());
        bytes.extend_from_slice(&mapped[..end]);
    } else {
        handle
            .by_ref()
            .take(wanted)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
            })?;
    }
    let complete = within && count(bytes.len()) <= limit;
    Ok((bytes, complete))
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

fn limit_error(path: &AbsPath, size: u64, limit: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "`{path}` has {size} bytes, more than the {limit} bytes read as text: search it with `search` instead"
        ),
    )
    .with_expected(serde_json::json!(limit))
    .with_actual(serde_json::json!(size))
}

/// Records of one text file: `content`, or `item` records for `key`; none
/// when `known` names the current version.
///
/// The text is stored in `cache` for later `since` reads.
///
/// # Errors
///
/// Fails with the errors of the selectors and services: `E_KEY_NOT_FOUND`
/// for a missing section, key or cell, `E_SYMBOL_NOT_FOUND` and
/// `E_SYMBOL_AMBIGUOUS` for symbols, `E_UNSUPPORTED_LANGUAGE`,
/// `E_UNSUPPORTED` and `E_FORMAT` for content the services cannot read.
pub(crate) fn read_source(
    ctx: &ExecCtx,
    cache: &dyn ReadCache,
    source: &TextSource,
    selector: &Selector,
    options: &ReadOptions,
) -> Result<Vec<Record>> {
    let current = source.etag();
    let known = options
        .known
        .iter()
        .any(|(path, tag)| *path == source.path && tag.matches(&current));
    if known {
        return Ok(Vec::new());
    }
    let now = ctx.clock().monotonic();
    let previous = options.since.as_ref().map(|tag| {
        tag.hash_prefix()
            .and_then(|prefix| cache.get(&source.path, &prefix, now))
    });
    cache.put(&source.path, &source.hash, &source.text, now);
    let lines = Lines::new(&source.text);
    if options
        .unchanged_since
        .as_ref()
        .is_some_and(|tag| tag.matches(&current))
    {
        let unchanged = Content {
            unchanged: Some(true),
            total_lines: Some(lines.total()),
            ..base_content(source)
        };
        return Ok(vec![Record::from(unchanged)]);
    }
    match previous {
        Some(Some(old)) => Ok(vec![Record::from(since_content(source, &lines, &old))]),
        Some(None) => select(ctx, source, &lines, selector, options)
            .map(|selected| selected.into_records(true)),
        None => select(ctx, source, &lines, selector, options)
            .map(|selected| selected.into_records(false)),
    }
}

/// Result of a selector.
enum Selected {
    Content(Box<Content>),
    Items(Vec<Item>),
}

impl Selected {
    fn into_records(self, since_missing: bool) -> Vec<Record> {
        match self {
            Self::Content(mut content) => {
                if since_missing {
                    content.since_missing = Some(true);
                }
                vec![Record::from(*content)]
            }
            Self::Items(items) => items.into_iter().map(Record::from).collect(),
        }
    }
}

fn select(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    selector: &Selector,
    options: &ReadOptions,
) -> Result<Selected> {
    let spans = match selector {
        Selector::Whole => return whole_content(ctx, source, lines, options).map(boxed),
        Selector::Cell(cell) => return notebook_content(source, lines, Some(cell)).map(boxed),
        Selector::Key(key) => return key_items(ctx, source, lines, key, options),
        Selector::Symbol(name) => {
            return symbol_content(ctx, source, lines, name, options).map(boxed);
        }
        Selector::Lines(ranges) => select::range_spans(lines, ranges)?,
        Selector::Last(count) => select::last_spans(lines, *count),
        Selector::Around(searcher) => select::around_spans(lines, searcher, options.context),
        Selector::Section(path) => vec![select::section_span(
            lines,
            path,
            &source.path.to_language_string(),
        )?],
    };
    let mut content = lines_content(ctx, source, lines, Some(spans), options);
    if options.view == View::OutlineFirst {
        attach_outline(ctx, source, &mut content, options)?;
    }
    Ok(Selected::Content(Box::new(content)))
}

fn boxed(content: Content) -> Selected {
    Selected::Content(Box::new(content))
}

fn base_content(source: &TextSource) -> Content {
    Content {
        path: path_field(&source.path),
        raw_path: source.path.raw_bytes(),
        size: Size::from_bytes(source.size),
        etag: etag_text(&source.etag()),
        encoding: source.encoding.label.clone(),
        bom: source.encoding.bom,
        line_ending: record_line_ending(source.line_ending),
        ..Content::default()
    }
}

fn whole_content(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    options: &ReadOptions,
) -> Result<Content> {
    match options.view {
        View::Header => Ok(Content {
            total_lines: Some(lines.total()),
            view: Some(View::Header.as_str().to_owned()),
            ..base_content(source)
        }),
        View::Outline | View::Symbols => outline_content(ctx, source, lines, options),
        View::OutlineFirst => {
            let mut content = lines_content(ctx, source, lines, None, options);
            attach_outline(ctx, source, &mut content, options)?;
            Ok(content)
        }
        View::Full | View::Hex if is_notebook(&source.path) => {
            notebook_content(source, lines, None)
        }
        View::Full | View::Hex => Ok(lines_content(ctx, source, lines, None, options)),
    }
}

/// Text of the lines of `spans`, with numbers and shortened lines as set in
/// `options`.
struct Rendered {
    text: String,
    lines: u64,
    shortened: u64,
    cut: Vec<(u64, String)>,
}

fn render(lines: &Lines<'_>, spans: &[LineSpan], options: &ReadOptions) -> Rendered {
    let mut rendered = Rendered {
        text: String::new(),
        lines: 0,
        shortened: 0,
        cut: Vec::new(),
    };
    let width = spans
        .iter()
        .map(|span| span.last.to_string().len())
        .max()
        .unwrap_or(1);
    for &span in spans {
        rendered.lines += span.len();
        if !options.line_numbers && options.max_line.is_none() {
            rendered.text.push_str(lines.slice(span));
            continue;
        }
        for line in span.first..=span.last {
            if options.line_numbers {
                rendered
                    .text
                    .push_str(&format!("{line:>width$}{NUMBER_SEPARATOR}"));
            }
            let content = lines.content(line);
            match options.max_line.and_then(|max| shorten(content, max)) {
                Some(short) => {
                    rendered.text.push_str(&short);
                    rendered.shortened += 1;
                    rendered.cut.push((line, content.to_owned()));
                }
                None => rendered.text.push_str(content),
            }
            rendered.text.push_str(lines.ending(line));
        }
    }
    rendered
}

/// The line cut after `max` characters with a marker of the characters left
/// out, the marker `search` uses; `None` for a line that is not longer.
fn shorten(line: &str, max: usize) -> Option<String> {
    let total = line.chars().count();
    (total > max).then(|| {
        let kept: String = line.chars().take(max).collect();
        format!("{kept} [{} characters omitted]", total - max)
    })
}

fn lines_content(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    spans: Option<Vec<LineSpan>>,
    options: &ReadOptions,
) -> Content {
    let selected: Vec<LineSpan> = spans
        .clone()
        .unwrap_or_else(|| lines.whole().into_iter().collect());
    let rendered = render(lines, &selected, options);
    if rendered.shortened > 0 {
        let path = source.path.to_language_string();
        crate::common::continuation::offer_cut_lines(
            ctx,
            rendered
                .cut
                .iter()
                .map(|(no, text)| nurt_contracts::continuation::CutLine {
                    path: path.clone(),
                    no: *no,
                    text: text.clone(),
                })
                .collect(),
        );
        ctx.note(NotableEntry {
            count: Some(rendered.shortened),
            ..at_path(Notable::TruncatedLines, &source.path)
        });
    }
    let (range, ranges) = match spans.as_deref() {
        None => (None, None),
        Some([single]) => (Some(single.range()), None),
        Some(several) => (
            None,
            Some(several.iter().map(|span| span.range()).collect()),
        ),
    };
    Content {
        text: rendered.text,
        lines: rendered.lines,
        total_lines: Some(lines.total()),
        range,
        ranges,
        truncated: (rendered.shortened > 0).then_some(true),
        ..base_content(source)
    }
}

fn since_content(source: &TextSource, lines: &Lines<'_>, old: &str) -> Content {
    let path = path_field(&source.path);
    let hunks: Vec<Hunk> = unified_diff(old, &source.text, DIFF_CONTEXT, false)
        .iter()
        .enumerate()
        .map(|(index, hunk)| Hunk {
            raw_path: source.path.raw_bytes(),
            ..hunk_record(&path, index + 1, hunk)
        })
        .collect();
    Content {
        unchanged: hunks.is_empty().then_some(true),
        diff: Some(hunks),
        total_lines: Some(lines.total()),
        ..base_content(source)
    }
}

fn hunk_record(path: &value::Path, number: usize, hunk: &diff::Hunk) -> Hunk {
    Hunk {
        path: path.clone(),
        old_start: count(hunk.old_start),
        old_len: count(hunk.old_len),
        new_start: count(hunk.new_start),
        new_len: count(hunk.new_len),
        lines: hunk.prefixed_lines(),
        id: Some(format!("{}#{number}", path.as_str())),
        header: hunk.header.clone(),
        ..Hunk::default()
    }
}

fn note_fallback(ctx: &ExecCtx, path: &AbsPath, detail: &str) {
    ctx.note(NotableEntry {
        detail: Some(detail.to_owned()),
        ..at_path(Notable::SemanticFallback, path)
    });
}

/// Symbols of the file from the code service.
enum Outline {
    Symbols {
        symbols: Vec<SymbolInfo>,
        language: LanguageId,
    },
    Unavailable(&'static str),
}

fn outline(ctx: &ExecCtx, source: &TextSource, depth: Option<u32>) -> Result<Outline> {
    let Some(code) = ctx.code() else {
        return Ok(Outline::Unavailable(NO_CODE_SERVICE));
    };
    let bytes = source.text.as_bytes();
    let Some(language) = code.language_of(Some(source.path.as_path()), bytes) else {
        return Ok(Outline::Unavailable(NO_GRAMMAR));
    };
    let symbols = code.outline(bytes, &language, depth)?;
    Ok(Outline::Symbols { symbols, language })
}

fn outline_content(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    options: &ReadOptions,
) -> Result<Content> {
    let (symbols, language) = match outline(ctx, source, options.depth)? {
        Outline::Symbols { symbols, language } => (symbols, language),
        Outline::Unavailable(reason) => {
            note_fallback(ctx, &source.path, reason);
            return Ok(lines_content(ctx, source, lines, None, options));
        }
    };
    let text = if options.view == View::Outline {
        outline_text(&symbols, options.line_numbers)
    } else {
        String::new()
    };
    let etag = etag_text(&source.etag());
    let records = symbols
        .iter()
        .map(|symbol| symbol_record(source, &language, &etag, symbol))
        .collect();
    Ok(Content {
        lines: Lines::new(&text).total(),
        text,
        total_lines: Some(lines.total()),
        symbols: Some(records),
        view: Some(options.view.as_str().to_owned()),
        ..base_content(source)
    })
}

fn attach_outline(
    ctx: &ExecCtx,
    source: &TextSource,
    content: &mut Content,
    options: &ReadOptions,
) -> Result<()> {
    match outline(ctx, source, options.depth)? {
        Outline::Symbols { symbols, language } => {
            let etag = etag_text(&source.etag());
            content.symbols = Some(
                symbols
                    .iter()
                    .map(|symbol| symbol_record(source, &language, &etag, symbol))
                    .collect(),
            );
            content.view = Some(View::OutlineFirst.as_str().to_owned());
        }
        Outline::Unavailable(reason) => note_fallback(ctx, &source.path, reason),
    }
    Ok(())
}

/// One line per symbol, indented by nesting level: the signature, or the
/// kind and the name.
fn outline_text(symbols: &[SymbolInfo], line_numbers: bool) -> String {
    let width = symbols
        .iter()
        .map(|symbol| symbol.span.start.line.to_string().len())
        .max()
        .unwrap_or(1);
    let mut text = String::new();
    for symbol in symbols {
        if line_numbers {
            let line = symbol.span.start.line;
            text.push_str(&format!("{line:>width$}{NUMBER_SEPARATOR}"));
        }
        for _ in 0..symbol.depth {
            text.push_str(OUTLINE_INDENT);
        }
        match &symbol.signature {
            Some(signature) => text.push_str(signature),
            None => text.push_str(&format!(
                "{} {}",
                symbol_kind(symbol.kind).as_str(),
                symbol.name
            )),
        }
        text.push('\n');
    }
    text
}

/// Last line of a symbol: the end position lies after the last character,
/// so a definition that ends with its line feed ends on the line before.
fn last_line(symbol: &SymbolInfo) -> u32 {
    let end = symbol.span.end;
    if end.column == 1 && end.line > symbol.span.start.line {
        end.line - 1
    } else {
        end.line
    }
}

fn symbol_record(
    source: &TextSource,
    language: &LanguageId,
    etag: &str,
    symbol: &SymbolInfo,
) -> Symbol {
    Symbol {
        name: symbol.name.clone(),
        kind: symbol_kind(symbol.kind),
        path: path_field(&source.path),
        line: u64::from(symbol.span.start.line),
        column: u64::from(symbol.span.start.column),
        end_line: u64::from(last_line(symbol)),
        language: language.to_string(),
        source: SymbolSource::Syntax,
        etag: etag.to_owned(),
        end_column: Some(u64::from(symbol.span.end.column)),
        container: symbol.container.clone(),
        signature: symbol.signature.clone(),
        text: None,
        exported: symbol.exported,
        references: None,
    }
}

fn symbol_kind(kind: services::SymbolKind) -> record::SymbolKind {
    match kind {
        services::SymbolKind::Function => record::SymbolKind::Function,
        services::SymbolKind::Method => record::SymbolKind::Method,
        services::SymbolKind::Class => record::SymbolKind::Class,
        services::SymbolKind::Struct => record::SymbolKind::Struct,
        services::SymbolKind::Interface => record::SymbolKind::Interface,
        services::SymbolKind::Enum => record::SymbolKind::Enum,
        services::SymbolKind::Variable => record::SymbolKind::Variable,
        services::SymbolKind::Constant => record::SymbolKind::Constant,
        services::SymbolKind::Field => record::SymbolKind::Field,
        services::SymbolKind::Module => record::SymbolKind::Module,
        services::SymbolKind::Type => record::SymbolKind::Type,
        services::SymbolKind::Macro => record::SymbolKind::Macro,
        _ => record::SymbolKind::Other,
    }
}

fn symbol_content(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    name: &str,
    options: &ReadOptions,
) -> Result<Content> {
    let Some(code) = ctx.code() else {
        note_fallback(ctx, &source.path, NO_CODE_SERVICE);
        return Ok(lines_content(ctx, source, lines, None, options));
    };
    let bytes = source.text.as_bytes();
    let shown = source.path.to_language_string();
    let language = code
        .language_of(Some(source.path.as_path()), bytes)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedLanguage,
                format!("no grammar recognises `{shown}`: read lines or a section instead"),
            )
        })?;
    let found = code.find_symbol(bytes, &language, name)?;
    let symbol = match found.as_slice() {
        [symbol] => symbol,
        [] => {
            return Err(Error::new(
                ErrorCode::SymbolNotFound,
                format!(
                    "`{shown}` defines no symbol `{name}`: list its symbols with `view: symbols`"
                ),
            )
            .with_found(name));
        }
        several => return Err(ambiguous_symbol(&shown, name, several)),
    };
    let first = u64::from(symbol.span.start.line).max(1);
    let last = u64::from(last_line(symbol)).min(lines.total()).max(first);
    let mut content = lines_content(
        ctx,
        source,
        lines,
        Some(vec![LineSpan::new(first, last)]),
        options,
    );
    let etag = etag_text(&source.etag());
    content.symbols = Some(vec![symbol_record(source, &language, &etag, symbol)]);
    Ok(content)
}

fn ambiguous_symbol(shown: &str, name: &str, symbols: &[SymbolInfo]) -> Error {
    let candidates: Vec<String> = symbols
        .iter()
        .map(|symbol| match &symbol.container {
            Some(container) => format!("{container}.{}", symbol.name),
            None => symbol.name.clone(),
        })
        .collect();
    let example = candidates.first().cloned().unwrap_or_default();
    Error::new(
        ErrorCode::SymbolAmbiguous,
        format!(
            "`{name}` names {} symbols in `{shown}`: qualify it with its container, such as `{example}`",
            symbols.len()
        ),
    )
    .with_found(name)
    .with_expected(serde_json::json!(candidates))
}

fn key_items(
    ctx: &ExecCtx,
    source: &TextSource,
    lines: &Lines<'_>,
    key: &KeyPath,
    options: &ReadOptions,
) -> Result<Selected> {
    let Some(doc) = ctx.doc() else {
        note_fallback(ctx, &source.path, NO_DOC_SERVICE);
        return Ok(boxed(lines_content(ctx, source, lines, None, options)));
    };
    let bytes = source.text.as_bytes();
    let shown = source.path.to_language_string();
    let format = doc
        .detect_format(Some(source.path.as_path()), bytes)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::Unsupported,
                format!(
                    "`{shown}` is not a structured document of a known format: read lines instead"
                ),
            )
        })?;
    let items = doc.read_keys(bytes, &format, key, options.depth)?;
    if items.is_empty() {
        return Err(Error::new(
            ErrorCode::KeyNotFound,
            format!("`{shown}` has no key `{key}`"),
        )
        .with_found(key.to_string()));
    }
    let path = path_field(&source.path);
    let etag = etag_text(&source.etag());
    Ok(Selected::Items(
        items
            .into_iter()
            .map(|item| item_record(&path, format.as_str(), &etag, item))
            .collect(),
    ))
}

fn item_record(path: &value::Path, format: &str, etag: &str, item: DocItem) -> Item {
    let span = item.span;
    Item {
        path: path.clone(),
        format: format.to_owned(),
        key: item.key.to_string(),
        value_type: value_type(&item.value),
        value: item.value,
        etag: etag.to_owned(),
        line: span.map(|span| u64::from(span.start.line)),
        column: span.map(|span| u64::from(span.start.column)),
        end_line: span.map(|span| u64::from(span.end.line)),
        start: span.map(|span| span.bytes.start),
        end: span.map(|span| span.bytes.end),
        comment: item.comment,
        document: item.document.map(u64::from),
        anchor: item.anchor,
        alias_of: item.alias_of,
        tag: item.tag,
    }
}

fn value_type(value: &Value) -> ValueType {
    match value {
        Value::Null => ValueType::Null,
        Value::Bool(_) => ValueType::Bool,
        Value::Int(_) | Value::BigInt(_) => ValueType::Int,
        Value::Number(_) | Value::Decimal(_) => ValueType::Number,
        Value::List(_) => ValueType::List,
        Value::Map(_) | Value::Record(_) => ValueType::Map,
        Value::Date(_) | Value::Instant(_) => ValueType::Date,
        Value::Bytes(_) => ValueType::Bytes,
        _ => ValueType::Text,
    }
}

fn is_notebook(path: &AbsPath) -> bool {
    path.as_path()
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(NOTEBOOK_EXTENSION))
}

fn notebook_content(
    source: &TextSource,
    lines: &Lines<'_>,
    cell: Option<&CellRef>,
) -> Result<Content> {
    let shown = source.path.to_language_string();
    if !is_notebook(&source.path) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`cell` selects a cell of a notebook, and `{shown}` is not an `.ipynb` file"),
        ));
    }
    let notebook = nurt_notebook::cells_as_content(source.text.as_bytes())
        .map_err(|error| error.to_error())?;
    let total = notebook.cells.len();
    let cells: Vec<CellData> = notebook
        .cells
        .into_iter()
        .filter(|data| cell.is_none_or(|wanted| cell_matches(data, wanted)))
        .collect();
    if let Some(wanted) = cell
        && cells.is_empty()
    {
        let code = match wanted {
            CellRef::Index(_) => ErrorCode::OutOfRange,
            CellRef::Id(_) | CellRef::IndexAndId(..) => ErrorCode::KeyNotFound,
        };
        return Err(Error::new(
            code,
            format!(
                "`{shown}` has no cell {}: the notebook has {total} cells, from index 0",
                describe_cell(wanted)
            ),
        )
        .with_actual(serde_json::json!(total)));
    }
    let mut text = String::new();
    for data in &cells {
        text.push_str(&data.source);
        if !data.source.is_empty() && !data.source.ends_with('\n') {
            text.push('\n');
        }
    }
    let path = path_field(&source.path);
    let etag = etag_text(&source.etag());
    Ok(Content {
        lines: Lines::new(&text).total(),
        text,
        total_lines: Some(lines.total()),
        cells: Some(
            cells
                .into_iter()
                .map(|data| cell_record(&path, &etag, data))
                .collect(),
        ),
        ..base_content(source)
    })
}

fn cell_matches(data: &CellData, wanted: &CellRef) -> bool {
    let index_is = |index: i64| i64::try_from(data.index).is_ok_and(|own| own == index);
    let id_is = |id: &str| data.id.as_deref() == Some(id);
    match wanted {
        CellRef::Index(index) => index_is(*index),
        CellRef::Id(id) => id_is(id),
        CellRef::IndexAndId(index, id) => index_is(*index) && id_is(id),
    }
}

fn describe_cell(cell: &CellRef) -> String {
    match cell {
        CellRef::Index(index) => format!("at index {index}"),
        CellRef::Id(id) | CellRef::IndexAndId(_, id) => format!("with identifier `{id}`"),
    }
}

fn cell_record(path: &value::Path, etag: &str, data: CellData) -> Cell {
    Cell {
        path: path.clone(),
        index: count(data.index),
        cell_type: data.cell_type,
        source: data.source,
        etag: etag.to_owned(),
        id: data.id,
        outputs: data
            .outputs
            .map(|outputs| outputs.into_iter().map(Value::from_plain_json).collect()),
        outputs_summary: data.outputs_summary,
        execution_count: data.execution_count,
        metadata: data.metadata.map(|metadata| {
            metadata
                .into_iter()
                .map(|(name, value)| (name, Value::from_plain_json(value)))
                .collect()
        }),
    }
}

fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::record::CellType;
    use nurt_contracts::services::{
        ByteRange, ClassifiedSpan, CodeService, DocFormatId, DocFormatInfo, DocService,
        FunctionMetrics, LanguageInfo, SerializeOptions, Services, SymbolReference, SyntaxIssue,
        SyntaxMatch, SyntaxPattern, SyntaxToken, TextPosition, TextSpan,
    };
    use nurt_contracts::stage::Limits;
    use nurt_contracts::value::Range;
    use nurt_testkit::ctx::TestContext;
    use nurt_text::codec::Codec;

    use nurt_testkit::vfs::test_path;

    use super::super::cache::MemoryReadCache;
    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    fn text_source(test: &TestContext, path: &AbsPath) -> TextSource {
        match load(&test.ctx(), path, EncodingParam::Auto).unwrap() {
            Loaded::Text(source) => *source,
            Loaded::Binary => panic!("text expected"),
        }
    }

    fn read_with(
        test: &TestContext,
        ctx: &ExecCtx,
        path: &AbsPath,
        selector: &Selector,
        options: &ReadOptions,
    ) -> Result<Vec<Record>> {
        let cache = MemoryReadCache::default();
        read_source(ctx, &cache, &text_source(test, path), selector, options)
    }

    fn contents(records: Vec<Record>) -> Vec<Content> {
        records
            .into_iter()
            .map(|record| Content::try_from(record).unwrap())
            .collect()
    }

    fn one(records: Vec<Record>) -> Content {
        let mut list = contents(records);
        assert_eq!(list.len(), 1);
        list.remove(0)
    }

    fn lines_selector(ranges: &[(i64, i64)]) -> Selector {
        Selector::Lines(
            ranges
                .iter()
                .map(|&(start, end)| Range::new(Some(start), Some(end)).unwrap())
                .collect(),
        )
    }

    fn numbered(count: u64) -> String {
        (1..=count).map(|line| format!("line {line}\n")).collect()
    }

    #[test]
    fn whole_text_carries_metadata_and_a_hashed_etag() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\n");
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(content.text, "one\ntwo\n");
        assert_eq!(content.lines, 2);
        assert_eq!(content.total_lines, Some(2));
        assert_eq!(content.size, Size::from_bytes(8));
        assert_eq!(content.encoding, "utf-8");
        assert!(!content.bom);
        assert_eq!(content.line_ending, record::LineEnding::Lf);
        assert_eq!(content.range, None);
        let etag = parse_etag(&content.etag).unwrap();
        assert_eq!(etag.hash, Some(content_hash(b"one\ntwo\n")));
    }

    #[test]
    fn empty_files_have_no_lines() {
        let (test, path) = with_file("/data/empty.txt", b"");
        let ctx = test.ctx();
        let whole = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(
            (whole.text.as_str(), whole.lines, whole.total_lines),
            ("", 0, Some(0))
        );
        let last = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Last(3),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(last.lines, 0);
        assert_eq!(last.ranges, Some(Vec::new()));
    }

    #[test]
    fn a_missing_final_line_feed_is_kept() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\nc");
        let ctx = test.ctx();
        let last = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Last(2),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(last.text, "b\nc");
        assert_eq!(last.range, Some(Range::new(Some(2), Some(3)).unwrap()));
        let all = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Last(10),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(all.text, "a\nb\nc");
        assert_eq!(all.total_lines, Some(3));
    }

    #[test]
    fn several_ranges_are_reported_as_ranges() {
        let text = numbered(20);
        let (test, path) = with_file("/data/a.txt", text.as_bytes());
        let selector = lines_selector(&[(10, 12), (2, 3), (11, 14)]);
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &selector,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(
            content.text,
            "line 10\nline 11\nline 12\nline 13\nline 14\nline 2\nline 3\n"
        );
        assert_eq!(content.lines, 7);
        assert_eq!(
            content.ranges,
            Some(vec![
                Range::new(Some(10), Some(14)).unwrap(),
                Range::new(Some(2), Some(3)).unwrap()
            ])
        );
        assert_eq!(content.range, None);
    }

    #[test]
    fn line_numbers_are_aligned_and_crlf_is_kept() {
        let (test, path) = with_file(
            "/data/a.txt",
            b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\r\n",
        );
        let options = ReadOptions {
            line_numbers: true,
            ..ReadOptions::default()
        };
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &lines_selector(&[(9, 10)]),
            &options,
        )
        .unwrap());
        assert_eq!(content.text, " 9  i\r\n10  j\r\n");
        assert_eq!(content.line_ending, record::LineEnding::Crlf);
    }

    #[test]
    fn long_lines_are_shortened_and_noted() {
        let (test, path) = with_file("/data/a.txt", b"short\n0123456789abcdef\n");
        let ctx = test.ctx();
        let options = ReadOptions {
            max_line: Some(10),
            ..ReadOptions::default()
        };
        let content = one(read_with(&test, &ctx, &path, &Selector::Whole, &options).unwrap());
        assert_eq!(content.text, "short\n0123456789 [6 characters omitted]\n");
        assert_eq!(content.truncated, Some(true));
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::TruncatedLines);
        assert_eq!(notes[0].count, Some(1));
    }

    #[test]
    fn header_view_has_no_text() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\n");
        let options = ReadOptions {
            view: View::Header,
            ..ReadOptions::default()
        };
        let content =
            one(read_with(&test, &test.ctx(), &path, &Selector::Whole, &options).unwrap());
        assert_eq!(content.text, "");
        assert_eq!(content.total_lines, Some(2));
        assert_eq!(content.view.as_deref(), Some("header"));
        assert!(!content.etag.is_empty());
    }

    #[test]
    fn legacy_encodings_and_byte_order_marks_are_decoded() {
        let (test, path) = with_file("/data/pl.txt", b"\xbf\xf3\xb3ty \x9cwierszcz\r\n");
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(content.text, "\u{17c}\u{f3}\u{142}ty \u{15b}wierszcz\r\n");
        assert_eq!(content.encoding, "windows-1250");
        let (test, path) = with_file("/data/bom.txt", b"\xef\xbb\xbfx\n");
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(content.text, "x\n");
        assert!(content.bom);
    }

    #[test]
    fn a_named_encoding_that_does_not_decode_fails_with_its_position() {
        let (test, path) = with_file("/data/a.txt", b"ok \xff\n");
        let error = load(&test.ctx(), &path, EncodingParam::Codec(Codec::Utf8)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
        assert!(error.location.is_some());
    }

    #[test]
    fn binary_content_is_recognised() {
        let (test, path) = with_file("/data/a.bin", b"\x7fELF\x00\x01\x02\x00");
        assert!(matches!(
            load(&test.ctx(), &path, EncodingParam::Auto).unwrap(),
            Loaded::Binary
        ));
    }

    #[test]
    fn directories_and_large_text_are_rejected() {
        let (test, path) = with_file("/data/a.txt", b"0123456789\n");
        let error = load(&test.ctx(), &abs("/data"), EncodingParam::Auto).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("not_file"));
        let ctx = test
            .builder()
            .limits(Limits {
                max_file_bytes_text: 4,
                ..Limits::default()
            })
            .build();
        let error = load(&ctx, &path, EncodingParam::Auto).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
        test.fs()
            .write_file(&path, b"\x00\x01\x02\x03\x00\x00\x00\x00\x00\x00")
            .unwrap();
        assert!(matches!(
            load(&ctx, &path, EncodingParam::Auto).unwrap(),
            Loaded::Binary
        ));
    }

    #[test]
    fn tags_accept_etags_hashes_and_hash_prefixes() {
        let (test, path) = with_file("/data/a.txt", b"x\n");
        let source = text_source(&test, &path);
        let current = source.etag();
        let digits = hash_digits(&source.hash);
        assert!(
            TagMatch::parse(&etag_text(&current))
                .unwrap()
                .matches(&current)
        );
        assert!(
            TagMatch::parse(&source.hash.to_string())
                .unwrap()
                .matches(&current)
        );
        assert!(
            TagMatch::parse(&digits[..8].to_uppercase())
                .unwrap()
                .matches(&current)
        );
        assert!(!TagMatch::parse("00000000").unwrap().matches(&current));
        for wrong in ["1a2b3c", "not-an-etag", "zzzzzzzzz"] {
            let error = TagMatch::parse(wrong).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue);
        }
    }

    #[test]
    fn unchanged_files_are_returned_without_text() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\n");
        let source = text_source(&test, &path);
        let options = ReadOptions {
            unchanged_since: Some(TagMatch::Hash(hash_digits(&source.hash)[..12].to_owned())),
            ..ReadOptions::default()
        };
        let content =
            one(read_with(&test, &test.ctx(), &path, &Selector::Whole, &options).unwrap());
        assert_eq!(content.unchanged, Some(true));
        assert_eq!(content.text, "");
        assert_eq!(content.total_lines, Some(2));
        test.fs().write_file(&path, b"a\nB\n").unwrap();
        let changed =
            one(read_with(&test, &test.ctx(), &path, &Selector::Whole, &options).unwrap());
        assert_eq!(changed.unchanged, None);
        assert_eq!(changed.text, "a\nB\n");
    }

    #[test]
    fn known_files_are_skipped() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let source = text_source(&test, &path);
        let options = ReadOptions {
            known: vec![(path.clone(), TagMatch::Etag(source.etag()))],
            ..ReadOptions::default()
        };
        assert!(
            read_with(&test, &test.ctx(), &path, &Selector::Whole, &options)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn since_returns_the_difference_against_the_cached_version() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\nthree\n");
        let ctx = test.ctx();
        let cache = MemoryReadCache::default();
        let first = text_source(&test, &path);
        one(read_source(
            &ctx,
            &cache,
            &first,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        test.fs().write_file(&path, b"one\n2\nthree\n").unwrap();
        let options = ReadOptions {
            since: Some(TagMatch::Hash(hash_digits(&first.hash)[..8].to_owned())),
            ..ReadOptions::default()
        };
        let second = text_source(&test, &path);
        let content = one(read_source(&ctx, &cache, &second, &Selector::Whole, &options).unwrap());
        assert_eq!(content.text, "");
        assert_eq!(content.unchanged, None);
        let hunks = content.diff.unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines, [" one", "-two", "+2", " three"]);
        assert_eq!(
            hunks[0].id.as_deref(),
            Some(format!("{}#1", path.to_language_string()).as_str())
        );
        let again = TagMatch::Hash(hash_digits(&second.hash));
        let same = one(read_source(
            &ctx,
            &cache,
            &second,
            &Selector::Whole,
            &ReadOptions {
                since: Some(again),
                ..ReadOptions::default()
            },
        )
        .unwrap());
        assert_eq!(same.diff, Some(Vec::new()));
        assert_eq!(same.unchanged, Some(true));
    }

    #[test]
    fn since_without_a_cached_version_returns_the_content() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let options = ReadOptions {
            since: Some(TagMatch::Hash("0123456789".to_owned())),
            ..ReadOptions::default()
        };
        let content =
            one(read_with(&test, &test.ctx(), &path, &Selector::Whole, &options).unwrap());
        assert_eq!(content.since_missing, Some(true));
        assert_eq!(content.text, "a\n");
    }

    #[test]
    fn cached_versions_expire() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let ctx = test.ctx();
        let cache = MemoryReadCache::new(1024, Duration::from_secs(60));
        let source = text_source(&test, &path);
        read_source(
            &ctx,
            &cache,
            &source,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap();
        test.clock().advance(Duration::from_secs(61));
        let options = ReadOptions {
            since: Some(TagMatch::Etag(source.etag())),
            ..ReadOptions::default()
        };
        let content = one(read_source(&ctx, &cache, &source, &Selector::Whole, &options).unwrap());
        assert_eq!(content.since_missing, Some(true));
    }

    const RUST: &str = "fn alpha() {\n    1\n}\nfn beta() {}\nfn beta() {}\n";

    /// Code service that knows `.rs` files and finds every `fn name(` line;
    /// a function that does not end on its line ends at the next `}` line.
    struct LineCode;

    fn position(line: usize, column: usize) -> TextPosition {
        TextPosition {
            line: u32::try_from(line).unwrap(),
            column: u32::try_from(column).unwrap(),
        }
    }

    fn functions(text: &str) -> Vec<SymbolInfo> {
        let lines: Vec<&str> = text.lines().collect();
        let mut symbols = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let Some(rest) = line.strip_prefix("fn ") else {
                continue;
            };
            let name = rest.split('(').next().unwrap().to_owned();
            let end = if line.ends_with("{}") {
                position(index + 1, line.len() + 1)
            } else {
                let close = (index..lines.len()).find(|&i| lines[i] == "}").unwrap();
                position(close + 2, 1)
            };
            let start = position(index + 1, 1);
            let span = TextSpan {
                bytes: ByteRange::new(0, 0),
                start,
                end,
            };
            symbols.push(SymbolInfo {
                name,
                kind: services::SymbolKind::Function,
                container: None,
                depth: 0,
                span,
                name_span: span,
                signature: Some(line.trim_end_matches(['{', '}', ' ']).to_owned()),
                exported: None,
            });
        }
        symbols
    }

    fn unused<T>() -> Result<T> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "not used by these tests",
        ))
    }

    impl CodeService for LineCode {
        fn languages(&self) -> Vec<LanguageInfo> {
            Vec::new()
        }
        fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
            path.filter(|path| path.extension().is_some_and(|e| e == "rs"))
                .map(|_| LanguageId::new("rust").unwrap())
        }
        fn outline(
            &self,
            bytes: &[u8],
            _language: &LanguageId,
            _depth: Option<u32>,
        ) -> Result<Vec<SymbolInfo>> {
            Ok(functions(std::str::from_utf8(bytes).unwrap()))
        }
        fn find_symbol(
            &self,
            bytes: &[u8],
            _language: &LanguageId,
            name: &str,
        ) -> Result<Vec<SymbolInfo>> {
            Ok(functions(std::str::from_utf8(bytes).unwrap())
                .into_iter()
                .filter(|symbol| symbol.name == name)
                .collect())
        }
        fn enclosing(
            &self,
            _bytes: &[u8],
            _language: &LanguageId,
            _offset: u64,
        ) -> Result<Option<SymbolInfo>> {
            unused()
        }
        fn classify_spans(
            &self,
            _bytes: &[u8],
            _language: &LanguageId,
        ) -> Result<Vec<ClassifiedSpan>> {
            unused()
        }
        fn check_syntax(&self, _bytes: &[u8], _language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
            unused()
        }
        fn compile_pattern(
            &self,
            _pattern: &str,
            _language: &LanguageId,
        ) -> Result<Box<dyn SyntaxPattern>> {
            unused()
        }
        fn syntax_search(
            &self,
            _bytes: &[u8],
            _pattern: &dyn SyntaxPattern,
        ) -> Result<Vec<SyntaxMatch>> {
            unused()
        }
        fn render_template(&self, _found: &SyntaxMatch, _template: &str) -> Result<String> {
            unused()
        }
        fn references(
            &self,
            _bytes: &[u8],
            _language: &LanguageId,
            _name: &str,
        ) -> Result<Vec<SymbolReference>> {
            unused()
        }
        fn function_metrics(
            &self,
            _bytes: &[u8],
            _language: &LanguageId,
        ) -> Result<Vec<FunctionMetrics>> {
            unused()
        }
        fn tokens(&self, _bytes: &[u8], _language: &LanguageId) -> Result<Vec<SyntaxToken>> {
            unused()
        }
    }

    /// Document service that knows `.json` files and the key `name`.
    struct NameDoc;

    impl DocService for NameDoc {
        fn formats(&self) -> Vec<DocFormatInfo> {
            Vec::new()
        }
        fn detect_format(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
            path.filter(|path| path.extension().is_some_and(|e| e == "json"))
                .map(|_| DocFormatId::JSON)
        }
        fn read_keys(
            &self,
            _bytes: &[u8],
            _format: &DocFormatId,
            key: &KeyPath,
            _depth: Option<u32>,
        ) -> Result<Vec<DocItem>> {
            if key.to_string() != "name" {
                return Ok(Vec::new());
            }
            Ok(vec![DocItem {
                key: key.clone(),
                value: Value::Text("nurt".to_owned()),
                span: Some(TextSpan {
                    bytes: ByteRange::new(12, 18),
                    start: position(2, 11),
                    end: position(2, 17),
                }),
                comment: None,
                document: None,
                anchor: None,
                alias_of: None,
                tag: None,
            }])
        }
        fn check_syntax(&self, _bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
            Ok(Vec::new())
        }
        fn serialize_records(
            &self,
            _records: &[Record],
            _format: &DocFormatId,
            _options: &SerializeOptions,
        ) -> Result<Vec<u8>> {
            unused()
        }
    }

    fn with_services(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new().with_services(
            Services::new()
                .with_code(Arc::new(LineCode))
                .with_doc(Arc::new(NameDoc)),
        );
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    #[test]
    fn symbols_select_their_definition() {
        let (test, path) = with_services("/src/lib.rs", RUST.as_bytes());
        let content = one(read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Symbol("alpha".to_owned()),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(content.text, "fn alpha() {\n    1\n}\n");
        assert_eq!(content.range, Some(Range::new(Some(1), Some(3)).unwrap()));
        let symbols = content.symbols.unwrap();
        assert_eq!(symbols[0].name, "alpha");
        assert_eq!((symbols[0].line, symbols[0].end_line), (1, 3));
        assert_eq!(symbols[0].source, SymbolSource::Syntax);
    }

    #[test]
    fn missing_and_ambiguous_symbols_fail() {
        let (test, path) = with_services("/src/lib.rs", RUST.as_bytes());
        let ctx = test.ctx();
        let missing = read_with(
            &test,
            &ctx,
            &path,
            &Selector::Symbol("gamma".to_owned()),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::SymbolNotFound);
        let ambiguous = read_with(
            &test,
            &ctx,
            &path,
            &Selector::Symbol("beta".to_owned()),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(ambiguous.code(), ErrorCode::SymbolAmbiguous);
        assert_eq!(
            ambiguous.expected,
            Some(serde_json::json!(["beta", "beta"]))
        );
        let (test, path) = with_services("/src/notes.txt", b"fn x() {}\n");
        let unsupported = read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Symbol("x".to_owned()),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(unsupported.code(), ErrorCode::UnsupportedLanguage);
    }

    #[test]
    fn outline_views_list_the_symbols() {
        let (test, path) = with_services("/src/lib.rs", RUST.as_bytes());
        let ctx = test.ctx();
        let view = |view: View| ReadOptions {
            view,
            ..ReadOptions::default()
        };
        let outline =
            one(read_with(&test, &ctx, &path, &Selector::Whole, &view(View::Outline)).unwrap());
        assert_eq!(outline.text, "fn alpha()\nfn beta()\nfn beta()\n");
        assert_eq!(outline.symbols.as_ref().map(Vec::len), Some(3));
        assert_eq!(outline.view.as_deref(), Some("outline"));
        let symbols =
            one(read_with(&test, &ctx, &path, &Selector::Whole, &view(View::Symbols)).unwrap());
        assert_eq!(symbols.text, "");
        assert_eq!(symbols.symbols.as_ref().map(Vec::len), Some(3));
        let first = one(read_with(
            &test,
            &ctx,
            &path,
            &lines_selector(&[(4, 4)]),
            &view(View::OutlineFirst),
        )
        .unwrap());
        assert_eq!(first.text, "fn beta() {}\n");
        assert_eq!(first.symbols.as_ref().map(Vec::len), Some(3));
        assert_eq!(first.view.as_deref(), Some("outline_first"));
    }

    #[test]
    fn views_fall_back_to_the_text_without_code_analysis() {
        let test = TestContext::new().with_services(Services::new());
        let path = abs("/src/lib.rs");
        test.fs().write_file(&path, RUST.as_bytes()).unwrap();
        let ctx = test.ctx();
        let options = ReadOptions {
            view: View::Outline,
            ..ReadOptions::default()
        };
        let content = one(read_with(&test, &ctx, &path, &Selector::Whole, &options).unwrap());
        assert_eq!(content.text, RUST);
        assert_eq!(content.symbols, None);
        let symbol = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Symbol("alpha".to_owned()),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(symbol.text, RUST);
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 2);
        assert!(
            notes
                .iter()
                .all(|note| note.code == Notable::SemanticFallback)
        );
        let (test, path) = with_services("/src/notes.txt", b"plain\n");
        let ctx = test.ctx();
        let plain = one(read_with(&test, &ctx, &path, &Selector::Whole, &options).unwrap());
        assert_eq!(plain.text, "plain\n");
        assert_eq!(ctx.take_notes()[0].detail.as_deref(), Some(NO_GRAMMAR));
    }

    #[test]
    fn keys_give_item_records() {
        let (test, path) = with_services("/data/package.json", b"{\n  \"name\": \"nurt\"\n}\n");
        let ctx = test.ctx();
        let key = Selector::Key(KeyPath::parse("name").unwrap());
        let records = read_with(&test, &ctx, &path, &key, &ReadOptions::default()).unwrap();
        assert_eq!(records.len(), 1);
        let item = Item::try_from(records[0].clone()).unwrap();
        assert_eq!(item.key, "name");
        assert_eq!(item.format, "json");
        assert_eq!(item.value, Value::Text("nurt".to_owned()));
        assert_eq!(item.value_type, ValueType::Text);
        assert_eq!(
            (item.line, item.column, item.start),
            (Some(2), Some(11), Some(12))
        );
        let missing = Selector::Key(KeyPath::parse("version").unwrap());
        let error = read_with(&test, &ctx, &path, &missing, &ReadOptions::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::KeyNotFound);
        let (test, path) = with_services("/data/notes.txt", b"name = x\n");
        let error =
            read_with(&test, &test.ctx(), &path, &key, &ReadOptions::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
    }

    #[test]
    fn keys_fall_back_to_the_text_without_document_analysis() {
        let test = TestContext::new().with_services(Services::new());
        let path = abs("/data/a.json");
        test.fs().write_file(&path, b"{}\n").unwrap();
        let ctx = test.ctx();
        let key = Selector::Key(KeyPath::parse("name").unwrap());
        let content = one(read_with(&test, &ctx, &path, &key, &ReadOptions::default()).unwrap());
        assert_eq!(content.text, "{}\n");
        assert_eq!(ctx.take_notes()[0].detail.as_deref(), Some(NO_DOC_SERVICE));
    }

    const NOTEBOOK: &str = r##"{"cells": [
 {"cell_type": "markdown", "id": "intro", "metadata": {}, "source": "# Title"},
 {"cell_type": "code", "id": "calc", "execution_count": 1, "metadata": {}, "outputs": [], "source": ["x = 1\n", "x + 1"]}
], "metadata": {}, "nbformat": 4, "nbformat_minor": 5}
"##;

    #[test]
    fn notebooks_are_read_as_cells() {
        let (test, path) = with_file("/nb/a.ipynb", NOTEBOOK.as_bytes());
        let ctx = test.ctx();
        let whole = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(whole.text, "# Title\nx = 1\nx + 1\n");
        let cells = whole.cells.unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1].cell_type, CellType::Code);
        assert_eq!(cells[1].id.as_deref(), Some("calc"));
        let by_id = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Cell(CellRef::Id("calc".to_owned())),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(by_id.text, "x = 1\nx + 1\n");
        let by_index = one(read_with(
            &test,
            &ctx,
            &path,
            &Selector::Cell(CellRef::Index(0)),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(by_index.cells.unwrap()[0].source, "# Title");
        let missing = read_with(
            &test,
            &ctx,
            &path,
            &Selector::Cell(CellRef::Index(5)),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::OutOfRange);
        assert_eq!(missing.actual, Some(serde_json::json!(2)));
        let unknown_id = read_with(
            &test,
            &ctx,
            &path,
            &Selector::Cell(CellRef::Id("absent".to_owned())),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(unknown_id.code(), ErrorCode::KeyNotFound);
    }

    #[test]
    fn lines_beyond_the_end_are_out_of_range() {
        let (test, path) = with_file("/data/a.txt", b"a\nb\nc\n");
        let ctx = test.ctx();
        let error = read_with(
            &test,
            &ctx,
            &path,
            &lines_selector(&[(4, 9)]),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        let clipped = one(read_with(
            &test,
            &ctx,
            &path,
            &lines_selector(&[(2, 9)]),
            &ReadOptions::default(),
        )
        .unwrap());
        assert_eq!(clipped.text, "b\nc\n");
    }

    #[test]
    fn cells_need_a_notebook() {
        let (test, path) = with_file("/nb/a.txt", b"text\n");
        let error = read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Cell(CellRef::Index(0)),
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let (test, path) = with_file("/nb/broken.ipynb", b"{\"cells\": 1}");
        let error = read_with(
            &test,
            &test.ctx(),
            &path,
            &Selector::Whole,
            &ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
    }

    #[test]
    fn views_combine_only_with_line_selectors() {
        let lines = lines_selector(&[(1, 2)]);
        assert!(View::Full.combines_with(&lines));
        assert!(View::OutlineFirst.combines_with(&lines));
        assert!(!View::OutlineFirst.combines_with(&Selector::Symbol("a".to_owned())));
        assert!(!View::Header.combines_with(&lines));
        assert!(View::Header.combines_with(&Selector::Whole));
        for name in View::NAMES {
            assert_eq!(View::parse(name).map(View::as_str), Some(name));
        }
        assert_eq!(View::parse("tree"), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
