// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Response views of captured output.
//!
//! The lines of each stream pass, in this order, through decoding, line
//! cleaning, the `summary` and `progress` patterns, collapsing of repeated
//! lines (`dedupe`), line selection (`filter` with `context`, or the error
//! lines of `show: errors` and of `show: auto` after a failure) and the
//! `head` and `tail` limits. `quiet_on_success` and `show: none` leave the
//! text out and keep the counters, summary lines and progress.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::OutputShown;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::OutputStream;

use super::cursor::{Cursor, LineReader, Position};
use super::decode::{self, DETECT_SAMPLE_BYTES, LineUnit, TextEncoding};
use super::filter::{Dedupe, ErrorLines, Pattern, clean_line};
use super::progress::{self, Observer, Watch};
use super::spool::{Snapshot, Source, io_error};
use crate::iface::{OutputView, RenderedOutput, ShowMode};

/// Lines of a stream that `show: auto` returns whole after a failure;
/// longer output returns its error lines when it has any.
const SHORT_OUTPUT_LINES: u64 = 20;

/// Key `run.error_context`: lines of context around error lines.
const RUN_ERROR_CONTEXT: usize = 2;

/// Last lines returned by `show: summary`.
const SUMMARY_TAIL_LINES: usize = 5;

/// Bytes of one stream read by one incremental read.
const READ_BUDGET_BYTES: u64 = 1024 * 1024;

/// Line standing for the lines left out between the head and the tail.
const OMITTED_MARK: &str = "...";

/// Line separating groups of selected lines that are not adjacent.
const GROUP_SEPARATOR: &str = "--";

/// What a response covers.
#[derive(Debug, Clone, Copy)]
pub(super) enum Mode {
    /// The whole output of a run that ended with `exit_code`, or that still
    /// runs (`None`).
    Whole {
        /// Exit code of the program.
        exit_code: Option<i32>,
    },
    /// The complete lines written after a cursor.
    Since {
        /// Cursor of an earlier response; `None` reads from the start.
        from: Option<Cursor>,
    },
}

/// Line selection applied to one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lines {
    /// All lines.
    Plain,
    /// Lines matching `filter`, with context.
    Filter,
    /// Error lines with context; the last lines when there are none.
    Errors,
    /// Error lines with context for long output with error lines; all lines
    /// otherwise.
    AutoFailure,
    /// The last lines.
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShownLine {
    index: u64,
    text: String,
}

/// First and last lines of a sequence of unknown length.
#[derive(Debug)]
struct HeadTail {
    head_limit: usize,
    tail_limit: usize,
    head: Vec<ShownLine>,
    tail: VecDeque<ShownLine>,
    total: u64,
}

impl HeadTail {
    fn new(head_limit: usize, tail_limit: usize) -> Self {
        Self {
            head_limit,
            tail_limit,
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
        }
    }

    fn push(&mut self, line: ShownLine) {
        self.total += 1;
        if self.head.len() < self.head_limit {
            self.head.push(line);
        } else if self.tail_limit > 0 {
            if self.tail.len() == self.tail_limit {
                self.tail.pop_front();
            }
            self.tail.push_back(line);
        }
    }

    fn finish(self, grouped: bool) -> Window {
        Window {
            head: self.head,
            tail: self.tail.into(),
            total: self.total,
            grouped,
        }
    }
}

/// Lines kept for one stream.
#[derive(Debug, Default)]
struct Window {
    head: Vec<ShownLine>,
    tail: Vec<ShownLine>,
    total: u64,
    grouped: bool,
}

impl Window {
    fn omitted(&self) -> u64 {
        self.total - (self.head.len() + self.tail.len()) as u64
    }

    /// Text of the kept lines without blank lines at the start and the end;
    /// lines left out between the head and the tail are marked.
    fn text(&self) -> String {
        let mut items: Vec<Option<&ShownLine>> = self.head.iter().map(Some).collect();
        if self.omitted() > 0 && !self.head.is_empty() && !self.tail.is_empty() {
            items.push(None);
        }
        items.extend(self.tail.iter().map(Some));
        let blank = |item: &Option<&ShownLine>| item.is_some_and(|line| line.text.is_empty());
        let start = items
            .iter()
            .position(|item| !blank(item))
            .unwrap_or(items.len());
        let end = items
            .iter()
            .rposition(|item| !blank(item))
            .map_or(start, |last| last + 1);
        let mut parts: Vec<&str> = Vec::new();
        let mut previous: Option<u64> = None;
        for item in &items[start..end] {
            match item {
                None => {
                    parts.push(OMITTED_MARK);
                    previous = None;
                }
                Some(line) => {
                    if self.grouped && previous.is_some_and(|index| line.index > index + 1) {
                        parts.push(GROUP_SEPARATOR);
                    }
                    parts.push(&line.text);
                    previous = Some(line.index);
                }
            }
        }
        parts.join("\n")
    }
}

#[derive(Debug, Clone, Copy)]
enum Matcher<'a> {
    Pattern(&'a Pattern),
    Errors(&'static ErrorLines),
}

impl Matcher<'_> {
    fn matches(self, text: &str) -> bool {
        match self {
            Self::Pattern(pattern) => pattern.is_match(text),
            Self::Errors(lines) => lines.is_error(text),
        }
    }
}

/// Lines matching a pattern with `context` lines before and after each;
/// overlapping ranges are merged.
#[derive(Debug)]
struct ContextSelect<'a> {
    matcher: Matcher<'a>,
    context: usize,
    before: VecDeque<ShownLine>,
    after: usize,
    hits: u64,
    out: HeadTail,
}

impl<'a> ContextSelect<'a> {
    fn new(matcher: Matcher<'a>, context: usize, out: HeadTail) -> Self {
        Self {
            matcher,
            context,
            before: VecDeque::new(),
            after: 0,
            hits: 0,
            out,
        }
    }

    fn push(&mut self, line: ShownLine) {
        if self.matcher.matches(&line.text) {
            self.hits += 1;
            for earlier in self.before.drain(..) {
                self.out.push(earlier);
            }
            self.out.push(line);
            self.after = self.context;
        } else if self.after > 0 {
            self.after -= 1;
            self.out.push(line);
        } else if self.context > 0 {
            if self.before.len() == self.context {
                self.before.pop_front();
            }
            self.before.push_back(line);
        }
    }
}

/// Processing of the lines of one shown stream.
struct Pipeline<'a> {
    lines: Lines,
    dedupe: Dedupe,
    plain: Option<HeadTail>,
    select: Option<ContextSelect<'a>>,
    scanned: u64,
    emitted: u64,
}

impl<'a> Pipeline<'a> {
    fn new(
        lines: Lines,
        limits: (usize, usize),
        view: &OutputView,
        matchers: Matchers<'a>,
    ) -> Self {
        let (head, tail) = limits;
        let errors =
            |out| ContextSelect::new(Matcher::Errors(matchers.errors), RUN_ERROR_CONTEXT, out);
        let (plain, select) = match (lines, matchers.filter) {
            (Lines::Filter, Some(filter)) => {
                let context = usize::try_from(view.context).unwrap_or(usize::MAX);
                let out = HeadTail::new(head, tail);
                (
                    None,
                    Some(ContextSelect::new(Matcher::Pattern(filter), context, out)),
                )
            }
            (Lines::Errors, _) => (
                Some(HeadTail::new(0, tail)),
                Some(errors(HeadTail::new(head, tail))),
            ),
            (Lines::AutoFailure, _) => (
                Some(HeadTail::new(head, tail)),
                Some(errors(HeadTail::new(head, tail))),
            ),
            (Lines::Summary, _) => (Some(HeadTail::new(0, SUMMARY_TAIL_LINES)), None),
            (Lines::Plain | Lines::Filter, _) => (Some(HeadTail::new(head, tail)), None),
        };
        Self {
            lines,
            dedupe: Dedupe::new(view.dedupe),
            plain,
            select,
            scanned: 0,
            emitted: 0,
        }
    }

    fn push(&mut self, text: String) {
        self.scanned += 1;
        if let Some(line) = self.dedupe.push(text) {
            self.emit(line);
        }
    }

    fn emit(&mut self, text: String) {
        let line = ShownLine {
            index: self.emitted,
            text,
        };
        self.emitted += 1;
        match (&mut self.select, &mut self.plain) {
            (Some(select), Some(plain)) => {
                select.push(line.clone());
                plain.push(line);
            }
            (Some(select), None) => select.push(line),
            (None, Some(plain)) => plain.push(line),
            (None, None) => {}
        }
    }

    fn finish(mut self) -> OutputShownText {
        if let Some(line) = self.dedupe.finish() {
            self.emit(line);
        }
        let hits = self.select.as_ref().map_or(0, |select| select.hits);
        let selected = match self.lines {
            Lines::Filter => true,
            Lines::Errors => hits > 0,
            Lines::AutoFailure => hits > 0 && self.emitted > SHORT_OUTPUT_LINES,
            Lines::Plain | Lines::Summary => false,
        };
        let window = if selected {
            self.select.map(|select| select.out.finish(true))
        } else {
            self.plain.map(|plain| plain.finish(false))
        }
        .unwrap_or_default();
        OutputShownText {
            text: window.text(),
            shown: OutputShown {
                head: window.head.len() as u64,
                tail: window.tail.len() as u64,
                omitted_lines: window.omitted(),
                filtered: self.scanned - window.total,
            },
        }
    }
}

/// Text and description of what is shown of one stream.
struct OutputShownText {
    text: String,
    shown: OutputShown,
}

#[derive(Debug, Clone, Copy)]
struct Matchers<'a> {
    filter: Option<&'a Pattern>,
    errors: &'static ErrorLines,
}

/// Compiled parameters of a view.
struct Compiled {
    filter: Option<Pattern>,
    watch: Watch,
    encoding: Option<TextEncoding>,
    errors: &'static ErrorLines,
}

impl Compiled {
    fn new(view: &OutputView) -> Result<Self> {
        let compile = |parameter, source: &String| Pattern::compile(parameter, source);
        let summary = view
            .summary
            .iter()
            .map(|source| compile("summary", source))
            .collect::<Result<Vec<_>>>()?;
        let progress = view
            .progress
            .as_ref()
            .map(|source| compile("progress", source))
            .transpose()?;
        Ok(Self {
            filter: view
                .filter
                .as_ref()
                .map(|source| compile("filter", source))
                .transpose()?,
            watch: Watch::new(summary, progress),
            encoding: decode::requested(view.encoding.as_deref())?,
            errors: ErrorLines::builtin()?,
        })
    }

    fn matchers(&self) -> Matchers<'_> {
        Matchers {
            filter: self.filter.as_ref(),
            errors: self.errors,
        }
    }
}

/// Result of reading one stream file.
struct Scan {
    end: Position,
    length: u64,
    shortened: u64,
}

/// Builds the response for `mode` from the spool state `snapshot`.
///
/// # Errors
///
/// `E_REGEX` for an invalid pattern, `E_INVALID_VALUE` for an unknown
/// encoding or a cursor past the end of the output, and I/O errors of the
/// spool files.
pub(super) fn respond(
    snapshot: &Snapshot,
    view: &OutputView,
    mode: Mode,
) -> Result<RenderedOutput> {
    let compiled = Compiled::new(view)?;
    let start = match mode {
        Mode::Since { from: Some(cursor) } => cursor,
        Mode::Since { from: None } | Mode::Whole { .. } => Cursor::default(),
    };
    let mut output = base_output(snapshot);
    let mut cursor = start;
    let mut observers: Vec<Observer<'_>> = Vec::new();
    let mut detected = Vec::new();
    let mut shortened = 0;
    for source in &snapshot.sources {
        let from = start.position(source.stream);
        let encoding = match compiled.encoding {
            Some(encoding) => encoding,
            None => detect_file(source)?,
        };
        let mut pipeline = plan(snapshot, view, mode, source.stream).map(|lines| {
            Pipeline::new(
                lines,
                limits(snapshot, view, mode),
                view,
                compiled.matchers(),
            )
        });
        let mut observer = compiled
            .watch
            .is_active()
            .then(|| compiled.watch.observer());
        let budget = matches!(mode, Mode::Since { .. }).then_some(READ_BUDGET_BYTES);
        let scan = scan(
            source,
            from,
            encoding,
            budget,
            pipeline.as_mut(),
            observer.as_mut(),
        )?;
        // A stream left out by `show` keeps its position, so that a later
        // read of that stream starts where the previous read of it ended.
        let left_out =
            matches!(mode, Mode::Since { .. }) && pipeline.is_none() && view.show != ShowMode::None;
        if !left_out {
            cursor.set_position(source.stream, scan.end);
        }
        shortened += scan.shortened;
        let covered = scan.end.offset.saturating_sub(from.offset);
        match source.stream {
            OutputStream::Stdout => output.stdout_read = covered,
            OutputStream::Stderr => output.stderr_read = covered,
        }
        if encoding.unit() != LineUnit::Byte
            && from == Position::default()
            && source.ended
            && !snapshot.merged
            && scan.end.offset >= scan.length
        {
            set_lines(&mut output, source.stream, scan.end.line);
        }
        if let Some(pipeline) = pipeline {
            let shown = if encoding == TextEncoding::Binary {
                binary_text(scan.end.offset - from.offset, scan.end.line - from.line)
            } else {
                pipeline.finish()
            };
            place(&mut output, source.stream, shown);
        }
        observers.extend(observer);
        detected.push((source, encoding, scan.length));
    }
    let findings = progress::combine(observers);
    output.summary_lines = findings.summary_lines;
    output.progress = findings.progress;
    output.decoded_as = decoded_as(&detected, compiled.encoding);
    output.notes = notes(snapshot, &detected, shortened);
    output.output_cursor = Some(cursor.encode(&snapshot.tag));
    Ok(output)
}

fn base_output(snapshot: &Snapshot) -> RenderedOutput {
    let [stdout, stderr] = snapshot.counters;
    let path = |stream| {
        snapshot
            .sources
            .iter()
            .find(|source| source.stream == stream)
            .map(|source| source.path.clone())
    };
    let mut output = RenderedOutput {
        stdout_bytes: stdout.bytes,
        stderr_bytes: stderr.bytes,
        stdout_lines: stdout.lines(),
        stderr_lines: stderr.lines(),
        output_sha256: snapshot.sha256.clone(),
        ..RenderedOutput::default()
    };
    if snapshot.merged {
        output.output_file = path(OutputStream::Stdout);
    } else {
        output.stdout_file = path(OutputStream::Stdout);
        output.stderr_file = path(OutputStream::Stderr);
    }
    output
}

fn set_lines(output: &mut RenderedOutput, stream: OutputStream, lines: u64) {
    match stream {
        OutputStream::Stdout => output.stdout_lines = lines,
        OutputStream::Stderr => output.stderr_lines = lines,
    }
}

fn place(output: &mut RenderedOutput, stream: OutputStream, shown: OutputShownText) {
    match stream {
        OutputStream::Stdout => {
            output.stdout = shown.text;
            output.shown.stdout = Some(shown.shown);
        }
        OutputStream::Stderr => {
            output.stderr = shown.text;
            output.shown.stderr = Some(shown.shown);
        }
    }
}

fn binary_text(bytes: u64, lines: u64) -> OutputShownText {
    OutputShownText {
        text: if bytes > 0 {
            format!("binary output, {bytes} bytes")
        } else {
            String::new()
        },
        shown: OutputShown {
            head: 0,
            tail: 0,
            omitted_lines: lines,
            filtered: 0,
        },
    }
}

/// Line selection of `stream`, or `None` when the stream is not shown.
fn plan(snapshot: &Snapshot, view: &OutputView, mode: Mode, stream: OutputStream) -> Option<Lines> {
    let failed = match mode {
        Mode::Whole { exit_code } => exit_code != Some(0),
        Mode::Since { .. } => false,
    };
    let hidden = view.show == ShowMode::None
        || (matches!(mode, Mode::Whole { .. }) && view.quiet_on_success && !failed);
    let shown = match mode {
        Mode::Whole { .. } => shows(snapshot, view.show, stream, failed),
        Mode::Since { .. } => {
            snapshot.merged
                || !matches!(
                    (view.show, stream),
                    (ShowMode::Stdout, OutputStream::Stderr)
                        | (ShowMode::Stderr, OutputStream::Stdout)
                )
        }
    };
    if hidden || !shown {
        return None;
    }
    let lines = match (mode, view.show, stream) {
        (Mode::Since { .. }, _, _) => Lines::Plain,
        (_, ShowMode::Summary, _) => return Some(Lines::Summary),
        (_, ShowMode::Errors, OutputStream::Stdout) => Lines::Errors,
        (_, ShowMode::Auto, _) if failed => Lines::AutoFailure,
        _ => Lines::Plain,
    };
    Some(if view.filter.is_some() {
        Lines::Filter
    } else {
        lines
    })
}

/// Whether `show` includes `stream`. Merged output has one stream, shown
/// as standard output.
fn shows(snapshot: &Snapshot, show: ShowMode, stream: OutputStream, failed: bool) -> bool {
    if snapshot.merged {
        return true;
    }
    match show {
        ShowMode::Auto | ShowMode::Summary => {
            stream == OutputStream::Stdout || failed || snapshot.counters[0].bytes == 0
        }
        ShowMode::Both | ShowMode::Errors => true,
        ShowMode::Stdout => stream == OutputStream::Stdout,
        ShowMode::Stderr => stream == OutputStream::Stderr,
        ShowMode::None => false,
    }
}

/// Numbers of first and last lines kept. An incremental read without
/// `head` and `tail` returns every new line.
fn limits(snapshot: &Snapshot, view: &OutputView, mode: Mode) -> (usize, usize) {
    let count = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
    match (mode, view.head, view.tail) {
        (Mode::Whole { .. }, head, tail) => (
            count(head.unwrap_or(snapshot.head)),
            count(tail.unwrap_or(snapshot.tail)),
        ),
        (Mode::Since { .. }, None, None) => (usize::MAX, 0),
        (Mode::Since { .. }, head, tail) => (head.map_or(0, count), tail.map_or(0, count)),
    }
}

fn detect_file(source: &Source) -> Result<TextEncoding> {
    let mut sample = Vec::new();
    File::open(&source.path)
        .and_then(|file| {
            file.take(DETECT_SAMPLE_BYTES as u64)
                .read_to_end(&mut sample)
        })
        .map_err(|error| io_error("read", &source.path, &error))?;
    let whole = source.ended && sample.len() < DETECT_SAMPLE_BYTES;
    Ok(decode::detect(&sample, whole))
}

/// Reads the lines of `source` from `from`, up to `budget` bytes, into the
/// pipeline and the observer.
fn scan(
    source: &Source,
    from: Position,
    encoding: TextEncoding,
    budget: Option<u64>,
    mut pipeline: Option<&mut Pipeline<'_>>,
    mut observer: Option<&mut Observer<'_>>,
) -> Result<Scan> {
    let read_error = |error: std::io::Error| io_error("read", &source.path, &error);
    let length = std::fs::metadata(&source.path).map_err(read_error)?.len();
    if from.offset > length {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "since: the cursor points past the end of the output; read again without since",
        ));
    }
    let keep = encoding != TextEncoding::Binary && (pipeline.is_some() || observer.is_some());
    let mut reader =
        LineReader::open(&source.path, from, encoding.unit(), source.ended).map_err(read_error)?;
    let mut shortened = 0;
    while budget.is_none_or(|budget| reader.position().offset - from.offset < budget) {
        let Some(line) = reader.next_line(keep).map_err(read_error)? else {
            break;
        };
        if !keep {
            continue;
        }
        shortened += u64::from(line.shortened);
        let text = clean_line(&encoding.decode(&line.bytes));
        if let Some(observer) = observer.as_mut() {
            observer.observe(&text);
        }
        if let Some(pipeline) = pipeline.as_mut() {
            pipeline.push(text);
        }
    }
    Ok(Scan {
        end: reader.position(),
        length,
        shortened,
    })
}

/// Encoding reported for the output: `binary` when a stream with bytes is
/// binary, else the encoding of the first stream with bytes.
fn decoded_as(
    detected: &[(&Source, TextEncoding, u64)],
    requested: Option<TextEncoding>,
) -> String {
    let mut written = detected
        .iter()
        .filter(|(_, _, length)| *length > 0)
        .map(|(_, encoding, _)| *encoding);
    if written
        .clone()
        .any(|encoding| encoding == TextEncoding::Binary)
    {
        return TextEncoding::Binary.name();
    }
    written
        .next()
        .or(requested)
        .unwrap_or(TextEncoding::Utf8)
        .name()
}

fn notes(
    snapshot: &Snapshot,
    detected: &[(&Source, TextEncoding, u64)],
    shortened: u64,
) -> Vec<NotableEntry> {
    let mut notes: Vec<NotableEntry> = detected
        .iter()
        .filter(|(_, encoding, length)| *encoding == TextEncoding::Binary && *length > 0)
        .map(|(source, _, length)| NotableEntry {
            path: Some(source.path.display().to_string()),
            count: Some(*length),
            ..NotableEntry::new(Notable::OutputBinary)
        })
        .collect();
    if snapshot.dropped > 0 {
        notes.push(NotableEntry {
            count: Some(snapshot.dropped),
            detail: Some(format!(
                "{} bytes of output beyond run.spool_max were not kept",
                snapshot.dropped
            )),
            ..NotableEntry::new(Notable::SpoolLimit)
        });
    }
    if shortened > 0 {
        notes.push(NotableEntry {
            count: Some(shortened),
            ..NotableEntry::new(Notable::TruncatedLines)
        });
    }
    notes
}
