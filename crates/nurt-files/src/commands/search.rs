// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `search` command: search file content for literal text, regular
//! expressions or syntax patterns.
//!
//! The source form walks the given files and directories with the walker of
//! this crate (`glob`, `exclude`, `exclude_preset`, `hidden`, `gitignore`)
//! and keeps the files selected by `file_type` and `exclude_file_type`. The
//! stage form searches the files of input `entry` records (a directory is
//! walked as in the source form), the text of input `content` records and
//! the text of input `line` records.
//!
//! A file is read whole, through the mapping of the file system when it
//! offers one. Binary files are skipped and counted unless `binary: text` is
//! given. Text in an encoding other than UTF-8 is decoded before matching;
//! byte offsets in `match` records still count bytes of the file as stored.
//! A file larger than the text limit of the execution and not mapped by the
//! file system is searched as a stream of UTF-8 bytes.
//!
//! With `output: matches` every matching line is one `match` record with the
//! position of its first match; with `only_matching` every match is one
//! record. `output: counts` gives one record per file (per pattern with a
//! list of patterns) whose `count` is the number of matches, `files` and
//! `unmatched` give `entry` records with `match_count`.
//!
//! Results follow `sort`: `path` keeps the order of the walk (roots in the
//! order given, then depth first by name) or of the input records and is
//! produced lazily; `modified`, `count` and `rank` search every file first.
//! Files are searched in parallel by up to [`Limits::threads`] threads; the
//! order of the results does not depend on the number of threads.
//!
//! `scope`, `enclosing` and `mode: syntax` use the code service of the
//! execution context. Without the service `scope` other than `any` and
//! `mode: syntax` fail with `E_UNSUPPORTED`, and `enclosing` adds no
//! `container`. A file whose language is unknown counts as code only.
//!
//! [`Limits::threads`]: nurt_contracts::stage::Limits::threads

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow, FsError, FsErrorKind, ReadHandle};
use nurt_contracts::record::{
    self, Content, Entry, EntryType, Line, Match, MatchContainer, RegexEngine,
};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ClassifiedSpan, CodeService, LanguageId, SpanClass, SymbolKind, SyntaxMatch, SyntaxPattern,
    TextCodecService, TextEncoding,
};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_text::codec::decode;
use nurt_text::search::{self as text_search, Hit, Pattern, SearchOptions, Searcher, Summary};

use super::file_types::TypeFilter;
use super::lines::{OffsetMapper, record_etag};
use crate::common::entry::entry_record;
use crate::common::notable::counted;
use crate::common::params::{WalkFilters, walk_filters};
use crate::common::path::{path_field, path_list_arg, record_path, resolve};
use crate::common::text::text_codec;
use crate::walk::{KindFilter, WalkItem, WalkOptions, Walker};

/// Name of the command.
const COMMAND: &str = "search";

const PATTERN: &str = "pattern";
const PATH: &str = "path";
const GLOB: &str = "glob";
const MODE: &str = "mode";
const CASE_SENSITIVE: &str = "case_sensitive";
const WHOLE_WORD: &str = "whole_word";
const MULTILINE: &str = "multiline";
const INVERT: &str = "invert";
const ONLY_MATCHING: &str = "only_matching";
const TRIM: &str = "trim";
const DEDUPE: &str = "dedupe";
const SCOPE: &str = "scope";
const ENCLOSING: &str = "enclosing";
const MAX_LINE: &str = "max_line";
const CONTEXT: &str = "context";
const BEFORE: &str = "before";
const AFTER: &str = "after";
const OUTPUT: &str = "output";
const MAX_PER_FILE: &str = "max_per_file";
const BINARY: &str = "binary";
const WHITESPACE: &str = "whitespace";
const LANGUAGE: &str = "language";
const FILE_TYPE: &str = "file_type";
const EXCLUDE_FILE_TYPE: &str = "exclude_file_type";
const SORT: &str = "sort";
const ADAPTIVE: &str = "adaptive";

/// Default of `glob`: every file.
const DEFAULT_GLOB: &str = "**";
/// Default of `max_line`, in characters.
const DEFAULT_MAX_LINE: usize = 500;
/// Files handed to the threads at once, per thread.
const BATCH_PER_THREAD: usize = 16;
/// Bytes examined for binary content before a file is searched as a stream.
const STREAM_PREFIX: usize = 64 * 1024;

/// Fields of `match` records filled from capture groups.
const GROUP_FIELDS: [&str; 2] = ["groups", "named"];

/// Stage of the `search` command.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Search;

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Search)]
}

impl StageImpl for Search {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![
                RecordKind::Entry,
                RecordKind::Content,
                RecordKind::Line,
            ]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Search {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let params = Params::read(args, ctx)?;
        let matcher = Matcher::new(&params)?;
        let run = Arc::new(Run::new(params, matcher, ctx));
        let source = input.is_none();
        let results: ResultIter<'a> = if source {
            let roots = path_list_arg(args, PATH, ctx)?;
            if roots.is_empty() {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    format!("`{COMMAND}` needs files or directories: give `{PATH}`"),
                ));
            }
            let jobs = run.collect_jobs(&roots)?;
            Box::new(FileScanner::new(Arc::clone(&run), jobs))
        } else {
            Box::new(InputScanner::new(Arc::clone(&run), input.into_records()?))
        };
        let mut emitter = Emitter::new(run, results);
        if source && emitter.collects_all() {
            emitter.prepare_all()?;
        }
        Ok(StageOutput::records(RecordStream::new(emitter)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Auto,
    Literal,
    Regex,
    Syntax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    Matches,
    Files,
    Counts,
    Unmatched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    Code,
    Comments,
    Strings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whitespace {
    Exact,
    Trailing,
    Indent,
    Normalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Path,
    Modified,
    Count,
    Rank,
}

const MODES: [(&str, Mode); 4] = [
    ("auto", Mode::Auto),
    ("literal", Mode::Literal),
    ("regex", Mode::Regex),
    ("syntax", Mode::Syntax),
];
const OUTPUTS: [(&str, Output); 4] = [
    ("matches", Output::Matches),
    ("files", Output::Files),
    ("counts", Output::Counts),
    ("unmatched", Output::Unmatched),
];
const SCOPES: [(&str, Scope); 4] = [
    ("any", Scope::Any),
    ("code", Scope::Code),
    ("comments", Scope::Comments),
    ("strings", Scope::Strings),
];
const WHITESPACES: [(&str, Whitespace); 4] = [
    ("exact", Whitespace::Exact),
    ("trailing", Whitespace::Trailing),
    ("indent", Whitespace::Indent),
    ("normalize", Whitespace::Normalize),
];
const SORTS: [(&str, SortKey); 4] = [
    ("path", SortKey::Path),
    ("modified", SortKey::Modified),
    ("count", SortKey::Count),
    ("rank", SortKey::Rank),
];
const BINARIES: [(&str, bool); 2] = [("skip", false), ("text", true)];

/// One pattern of the search.
#[derive(Debug, Clone)]
struct PatternSpec {
    pattern: Pattern,
    /// The pattern is a regular expression as written.
    regex: bool,
    /// Source text, for syntax patterns.
    source: String,
}

/// Parameters of one execution.
#[derive(Debug, Clone)]
struct Params {
    patterns: Vec<PatternSpec>,
    listed: bool,
    mode: Mode,
    options: SearchOptions,
    multiline: bool,
    invert: bool,
    only_matching: bool,
    trim: bool,
    dedupe: bool,
    scope: Scope,
    enclosing: bool,
    language: Option<LanguageId>,
    before: usize,
    after: usize,
    max_line: usize,
    max_per_file: Option<u64>,
    output: Output,
    binary_text: bool,
    globs: Vec<String>,
    filters: WalkFilters,
    types: TypeFilter,
    sort: SortKey,
    adaptive: bool,
    groups: bool,
}

impl Params {
    fn read(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        let mode = choice(args, MODE, &MODES)?;
        let whitespace = choice(args, WHITESPACE, &WHITESPACES)?;
        let (patterns, listed) = read_patterns(args, mode, whitespace)?;
        let multiline = flag(args, MULTILINE, false)?
            || patterns
                .iter()
                .any(|spec| !spec.regex && mode != Mode::Syntax && spec.source.contains('\n'));
        let context = amount(args, CONTEXT)?.unwrap_or(0);
        let mut params = Self {
            listed,
            mode,
            multiline,
            invert: flag(args, INVERT, false)?,
            only_matching: flag(args, ONLY_MATCHING, false)?,
            trim: flag(args, TRIM, false)?,
            dedupe: flag(args, DEDUPE, false)?,
            scope: choice(args, SCOPE, &SCOPES)?,
            enclosing: flag(args, ENCLOSING, false)?,
            language: language_param(args)?,
            before: amount(args, BEFORE)?.unwrap_or(context),
            after: amount(args, AFTER)?.unwrap_or(context),
            max_line: amount(args, MAX_LINE)?.unwrap_or(DEFAULT_MAX_LINE),
            max_per_file: amount(args, MAX_PER_FILE)?
                .filter(|&limit| limit > 0)
                .map(|limit| u64::try_from(limit).unwrap_or(u64::MAX)),
            output: choice(args, OUTPUT, &OUTPUTS)?,
            binary_text: choice(args, BINARY, &BINARIES)?,
            globs: text_list(args, GLOB)?,
            filters: walk_filters(args)?,
            types: TypeFilter::new(
                &text_list(args, FILE_TYPE)?,
                &text_list(args, EXCLUDE_FILE_TYPE)?,
                FILE_TYPE,
                EXCLUDE_FILE_TYPE,
            )?,
            sort: choice(args, SORT, &SORTS)?,
            adaptive: flag(args, ADAPTIVE, false)?,
            groups: GROUP_FIELDS
                .iter()
                .any(|field| ctx.demand().fields.wants(field)),
            options: SearchOptions::new(),
            patterns,
        };
        if params.globs.is_empty() {
            params.globs.push(DEFAULT_GLOB.to_owned());
        }
        params.check(args, ctx)?;
        params.options = params.search_options(
            flag(args, CASE_SENSITIVE, true)?,
            flag(args, WHOLE_WORD, false)?,
        );
        Ok(params)
    }

    fn check(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<()> {
        let needs_code = self.scope != Scope::Any || self.mode == Mode::Syntax;
        if needs_code && ctx.code().is_none() {
            let name = if self.mode == Mode::Syntax {
                MODE
            } else {
                SCOPE
            };
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!(
                    "`{name}` needs the code command group, which is not loaded: search with `{SCOPE}: any` and a text pattern"
                ),
            ));
        }
        if self.invert && needs_code {
            let other = if self.mode == Mode::Syntax {
                MODE
            } else {
                SCOPE
            };
            return Err(Error::new(
                ErrorCode::ParamConflict,
                format!(
                    "`{}` cannot combine `{INVERT}` with `{other}`: leave out one of them",
                    args.command()
                ),
            ));
        }
        Ok(())
    }

    /// Whether `max_per_file` is applied after the matches are filtered by
    /// scope, instead of by the searcher.
    fn filters_matches(&self) -> bool {
        self.scope != Scope::Any
    }

    fn search_options(&self, case_sensitive: bool, whole_word: bool) -> SearchOptions {
        let searcher_limit = match self.output {
            _ if self.filters_matches() => None,
            Output::Unmatched => Some(1),
            _ => self.max_per_file,
        };
        SearchOptions::new()
            .case_sensitive(case_sensitive)
            .whole_word(whole_word)
            .multiline(self.multiline)
            .invert(self.invert)
            .only_matching(self.only_matching)
            .before(self.before)
            .after(self.after)
            .max_line(self.max_line)
            .max_per_file(searcher_limit)
    }
}

fn read_patterns(
    args: &BoundArgs,
    mode: Mode,
    whitespace: Whitespace,
) -> Result<(Vec<PatternSpec>, bool)> {
    let (items, listed) = match args.require(PATTERN)? {
        Value::List(items) => (items.iter().collect::<Vec<_>>(), true),
        value => (vec![value], false),
    };
    if items.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{COMMAND}` needs at least one pattern in `{PATTERN}`"),
        ));
    }
    let specs = items
        .into_iter()
        .map(|item| pattern_spec(item, mode, whitespace))
        .collect::<Result<Vec<_>>>()?;
    Ok((specs, listed))
}

fn pattern_spec(value: &Value, mode: Mode, whitespace: Whitespace) -> Result<PatternSpec> {
    let literal = |text: &str| match whitespace {
        Whitespace::Exact => Pattern::literal(text),
        tolerance => Pattern::regex(tolerant_regex(text, tolerance)),
    };
    let (pattern, regex, source) = match (value, mode) {
        (Value::Text(text) | Value::Enum(text), Mode::Regex) => {
            (Pattern::regex(text.as_str()), true, text)
        }
        (Value::Text(text) | Value::Enum(text), _) => (literal(text), false, text),
        (Value::Regex(regex), Mode::Literal) => (literal(&regex.pattern), false, &regex.pattern),
        (Value::Regex(_), Mode::Syntax) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("a syntax pattern of `{COMMAND}` is text, not a regular expression"),
            ));
        }
        (Value::Regex(regex), _) => (
            Pattern::regex_with_flags(regex.pattern.as_str(), regex.flags),
            true,
            &regex.pattern,
        ),
        _ => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{COMMAND}` expects text or a regular expression for `{PATTERN}`"),
            ));
        }
    };
    Ok(PatternSpec {
        pattern,
        regex,
        source: source.clone(),
    })
}

/// Regular expression of a literal that tolerates differences of spaces and
/// tabs: at the end of every line (`trailing`), at the start of every line
/// (`indent`), or in every run of white space (`normalize`).
fn tolerant_regex(text: &str, whitespace: Whitespace) -> String {
    const BLANKS: &str = "[ \\t]*";
    let lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .map(|line| match whitespace {
            Whitespace::Exact => escape(line),
            Whitespace::Trailing => format!("{}{BLANKS}", escape(line.trim_end())),
            Whitespace::Indent => format!("{BLANKS}{}", escape(line.trim_start())),
            Whitespace::Normalize => {
                let words: Vec<String> = line.split_whitespace().map(escape).collect();
                format!("{BLANKS}{}{BLANKS}", words.join("[ \\t]+"))
            }
        })
        .collect::<Vec<_>>();
    lines.join("\\r?\\n")
}

/// Escapes the characters with a meaning in regular expressions.
fn escape(text: &str) -> String {
    const SPECIAL: &str = "\\.+*?()|[]{}^$#&-~";
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if SPECIAL.contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn flag(args: &BoundArgs, name: &str, default: bool) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(default))
}

fn amount(args: &BoundArgs, name: &str) -> Result<Option<usize>> {
    match args.int(name)? {
        None => Ok(None),
        Some(number) if number >= 0 => Ok(Some(usize::try_from(number).unwrap_or(usize::MAX))),
        Some(number) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` cannot be negative, not {number}: give 0 or more"),
        )
        .with_found(number.to_string())),
    }
}

fn choice<T: Copy>(args: &BoundArgs, name: &str, table: &[(&str, T)]) -> Result<T> {
    let text = match args.value(name) {
        None | Some(Value::Null) => None,
        Some(Value::Text(text) | Value::Enum(text)) => Some(text.as_str()),
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`{}` expects a value of the enumeration for `{name}`",
                    args.command()
                ),
            ));
        }
    };
    let found = match text {
        None => table.first(),
        Some(text) => table.iter().find(|(label, _)| *label == text),
    };
    found.map(|(_, value)| *value).ok_or_else(|| {
        let labels: Vec<&str> = table.iter().map(|(label, _)| *label).collect();
        let given = text.unwrap_or_default();
        Error::new(
            ErrorCode::UnknownValue,
            format!(
                "`{given}` is not a value of `{name}`: use one of {}",
                labels.join(", ")
            ),
        )
        .with_found(given)
        .with_expected(serde_json::json!(labels))
    })
}

fn text_list(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let item = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) | Value::Enum(text) => Ok(text.clone()),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`{}` expects text or a list of texts for `{name}`",
                args.command()
            ),
        )),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(item).collect(),
        Some(value) => item(value).map(|text| vec![text]),
    }
}

fn language_param(args: &BoundArgs) -> Result<Option<LanguageId>> {
    let Some(text) = args.text(LANGUAGE)? else {
        return Ok(None);
    };
    LanguageId::new(text.trim().to_lowercase())
        .map(Some)
        .map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{text}` is not a language name: give a name such as rust or python"),
            )
            .with_found(text)
        })
}

/// Compiled patterns.
enum Matcher {
    Text {
        searcher: Searcher,
        engines: Vec<Option<RegexEngine>>,
    },
    Syntax(SyntaxSearch),
}

impl Matcher {
    fn new(params: &Params) -> Result<Self> {
        if params.mode == Mode::Syntax {
            return Ok(Self::Syntax(SyntaxSearch {
                sources: params
                    .patterns
                    .iter()
                    .map(|spec| spec.source.clone())
                    .collect(),
                compiled: Mutex::new(HashMap::new()),
            }));
        }
        let patterns: Vec<Pattern> = params
            .patterns
            .iter()
            .map(|spec| spec.pattern.clone())
            .collect();
        let searcher = Searcher::new(&patterns, params.options.clone()).map_err(|error| {
            let found = match &error {
                text_search::SearchError::Pattern { pattern, .. }
                | text_search::SearchError::TooLarge { pattern, .. } => params
                    .patterns
                    .get(*pattern)
                    .map(|spec| spec.source.clone()),
                _ => None,
            };
            let converted = Error::new(error.code(), error.to_string());
            match found {
                Some(source) => converted.with_found(source),
                None => converted,
            }
        })?;
        let engines = params
            .patterns
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                if spec.regex {
                    searcher.engine(index)
                } else {
                    None
                }
            })
            .collect();
        Ok(Self::Text { searcher, engines })
    }
}

type CompiledPatterns = Arc<Vec<Box<dyn SyntaxPattern>>>;

/// Syntax patterns, compiled once per language.
struct SyntaxSearch {
    sources: Vec<String>,
    compiled: Mutex<HashMap<String, CompiledPatterns>>,
}

impl SyntaxSearch {
    /// Patterns compiled for `language`. A pattern that does not compile
    /// fails when the language was given; for a detected language the file
    /// is not searched.
    fn compiled(
        &self,
        service: &dyn CodeService,
        language: &LanguageId,
        given: bool,
    ) -> Result<CompiledPatterns> {
        let mut cache = self.compiled.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(patterns) = cache.get(language.as_str()) {
            return Ok(Arc::clone(patterns));
        }
        let compiled: Result<Vec<_>> = self
            .sources
            .iter()
            .map(|source| service.compile_pattern(source, language))
            .collect();
        let patterns = match compiled {
            Ok(patterns) => Arc::new(patterns),
            Err(error) if given => return Err(error),
            Err(_) => Arc::new(Vec::new()),
        };
        cache.insert(language.as_str().to_owned(), Arc::clone(&patterns));
        Ok(patterns)
    }
}

/// File of the source form to search.
#[derive(Debug, Clone)]
struct FileJob {
    path: AbsPath,
    modified_ns: i64,
}

/// Match of one pattern, with offsets in the file.
#[derive(Debug, Clone, Default)]
struct FoundMatch {
    pattern: usize,
    start: u64,
    end: u64,
    line: u64,
    column: u64,
    end_line: u64,
    end_column: u64,
    text: String,
    engine: Option<RegexEngine>,
    groups: Option<Vec<Option<String>>>,
    named: Option<Vec<(String, Option<String>)>>,
    container: Option<MatchContainer>,
}

/// Line or block of lines with its matches; a line without matches with
/// `invert`.
#[derive(Debug, Clone, Default)]
struct Found {
    line: u64,
    start: u64,
    end: u64,
    text: String,
    before: Vec<String>,
    after: Vec<String>,
    matches: Vec<FoundMatch>,
}

/// Hits of one file or input record.
#[derive(Debug, Clone, Default)]
struct Scan {
    hits: Vec<Found>,
    per_pattern: Vec<u64>,
    matches: u64,
    truncated: u64,
    backtracked: u64,
}

impl Scan {
    fn new(patterns: usize) -> Self {
        Self {
            per_pattern: vec![0; patterns],
            ..Self::default()
        }
    }

    fn push(&mut self, found: Found) {
        for found_match in &found.matches {
            if let Some(count) = self.per_pattern.get_mut(found_match.pattern) {
                *count += 1;
            }
        }
        self.matches += byte_count(found.matches.len());
        self.hits.push(found);
    }

    fn merge(&mut self, other: Self) {
        for (total, count) in self.per_pattern.iter_mut().zip(other.per_pattern) {
            *total += count;
        }
        self.matches += other.matches;
        self.truncated += other.truncated;
        self.backtracked += other.backtracked;
        self.hits.extend(other.hits);
    }

    fn finish(&mut self, summary: &Summary) {
        self.truncated += summary.truncated_lines;
        self.backtracked += summary.backtrack_limit_hits;
    }

    /// Units counted by `max_per_file`: matches with `only_matching`,
    /// otherwise hits.
    fn limited_units(&self, only_matching: bool) -> u64 {
        if only_matching {
            self.matches
        } else {
            byte_count(self.hits.len())
        }
    }
}

/// Searched file or input record.
#[derive(Debug, Clone)]
struct FileResult {
    path: AbsPath,
    etag: String,
    modified_ns: i64,
    scan: Scan,
}

impl FileResult {
    /// Matches, or lines without a match with `invert`.
    fn total(&self, invert: bool) -> u64 {
        if invert {
            byte_count(self.scan.hits.len())
        } else {
            self.scan.matches
        }
    }

    fn patterns_matched(&self) -> usize {
        self.scan
            .per_pattern
            .iter()
            .filter(|&&count| count > 0)
            .count()
    }
}

/// What became of one file.
enum FileOutcome {
    Searched(FileResult),
    Binary,
    Denied,
    Vanished,
}

/// Conversion of offsets in the searched text to offsets in the file.
struct Placement<'t> {
    text: &'t str,
    mapper: OffsetMapper,
    line_base: u64,
}

impl<'t> Placement<'t> {
    fn shifted(shift: u64, line_base: u64) -> Self {
        Self {
            text: "",
            mapper: OffsetMapper::shifted(shift),
            line_base,
        }
    }

    fn decoded(text: &'t str, mapper: OffsetMapper, line_base: u64) -> Self {
        Self {
            text,
            mapper,
            line_base,
        }
    }

    fn offset(&mut self, offset: u64) -> u64 {
        self.mapper
            .map(self.text, usize::try_from(offset).unwrap_or(usize::MAX))
    }
}

/// Lexical classes and enclosing symbols of one searched text.
struct CodeView<'c> {
    service: &'c dyn CodeService,
    bytes: &'c [u8],
    language: Option<LanguageId>,
    spans: Option<Vec<ClassifiedSpan>>,
    scope: Scope,
    enclosing: bool,
}

impl CodeView<'_> {
    /// Whether a match starting at `offset` lies in the requested scope.
    fn accepts(&self, offset: u64) -> bool {
        let wanted = match self.scope {
            Scope::Any => return true,
            Scope::Code => SpanClass::Code,
            Scope::Comments => SpanClass::Comment,
            Scope::Strings => SpanClass::String,
        };
        let class = self
            .spans
            .as_deref()
            .map_or(SpanClass::Code, |spans| class_at(spans, offset));
        class == wanted
    }

    fn container(&self, offset: u64, line_base: u64) -> Option<MatchContainer> {
        if !self.enclosing {
            return None;
        }
        let language = self.language.as_ref()?;
        let symbol = self
            .service
            .enclosing(self.bytes, language, offset)
            .ok()
            .flatten()?;
        let name = match symbol.container {
            Some(container) => format!("{container}.{}", symbol.name),
            None => symbol.name,
        };
        Some(MatchContainer {
            name,
            kind: symbol_kind(symbol.kind),
            line: u64::from(symbol.span.start.line) + line_base,
            end_line: Some(u64::from(symbol.span.end.line) + line_base),
        })
    }
}

fn class_at(spans: &[ClassifiedSpan], offset: u64) -> SpanClass {
    let index = spans.partition_point(|span| span.bytes.end <= offset);
    spans
        .get(index)
        .filter(|span| span.bytes.contains(offset))
        .map_or(SpanClass::Code, |span| span.class)
}

fn symbol_kind(kind: SymbolKind) -> record::SymbolKind {
    match kind {
        SymbolKind::Function => record::SymbolKind::Function,
        SymbolKind::Method => record::SymbolKind::Method,
        SymbolKind::Class => record::SymbolKind::Class,
        SymbolKind::Struct => record::SymbolKind::Struct,
        SymbolKind::Interface => record::SymbolKind::Interface,
        SymbolKind::Enum => record::SymbolKind::Enum,
        SymbolKind::Variable => record::SymbolKind::Variable,
        SymbolKind::Constant => record::SymbolKind::Constant,
        SymbolKind::Field => record::SymbolKind::Field,
        SymbolKind::Module => record::SymbolKind::Module,
        SymbolKind::Type => record::SymbolKind::Type,
        SymbolKind::Macro => record::SymbolKind::Macro,
        _ => record::SymbolKind::Other,
    }
}

/// Start offsets of the lines of a text.
struct LineTable<'h> {
    bytes: &'h [u8],
    starts: Vec<usize>,
}

impl<'h> LineTable<'h> {
    fn new(bytes: &'h [u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(index, _)| index + 1)
                .filter(|&start| start < bytes.len()),
        );
        Self { bytes, starts }
    }

    fn count(&self) -> u64 {
        if self.bytes.is_empty() {
            0
        } else {
            byte_count(self.starts.len())
        }
    }

    /// Content of line `no`, from 1, without its line ending.
    fn line(&self, no: u64) -> &'h [u8] {
        let index = usize::try_from(no.saturating_sub(1)).unwrap_or(usize::MAX);
        let Some(&start) = self.starts.get(index) else {
            return &[];
        };
        let end = self
            .starts
            .get(index + 1)
            .map_or(self.bytes.len(), |next| next - 1);
        let line = self.bytes.get(start..end).unwrap_or_default();
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        line.strip_suffix(b"\r").unwrap_or(line)
    }
}

/// Text of a line shortened to `max` characters with a marker of the
/// characters left out; 0 keeps the line whole.
fn shorten(bytes: &[u8], max: usize) -> (String, bool) {
    let text = String::from_utf8_lossy(bytes);
    let total = text.chars().count();
    if max == 0 || total <= max {
        return (text.into_owned(), false);
    }
    let mut shown: String = text.chars().take(max).collect();
    shown.push_str(&format!(" [{} characters omitted]", total - max));
    (shown, true)
}

fn byte_count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Counters reported as notable entries when the stream ends.
#[derive(Debug, Default)]
struct Totals {
    binary: AtomicU64,
    denied: AtomicU64,
    truncated: AtomicU64,
    backtracked: AtomicU64,
}

/// State of one execution, shared by the producers of results and records.
struct Run<'a> {
    params: Params,
    matcher: Matcher,
    ctx: &'a ExecCtx,
    totals: Totals,
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        let entries = [
            (Notable::BinarySkipped, &self.totals.binary),
            (Notable::PermissionDeniedSkipped, &self.totals.denied),
            (Notable::TruncatedLines, &self.totals.truncated),
            (Notable::RegexBacktracking, &self.totals.backtracked),
        ];
        for (code, count) in entries {
            let count = count.load(Ordering::Relaxed);
            if count > 0 {
                self.ctx.note(counted(code, count));
            }
        }
    }
}

impl<'a> Run<'a> {
    fn new(params: Params, matcher: Matcher, ctx: &'a ExecCtx) -> Self {
        Self {
            params,
            matcher,
            ctx,
            totals: Totals::default(),
        }
    }

    fn pattern_count(&self) -> usize {
        self.params.patterns.len()
    }

    fn add(counter: &AtomicU64, count: u64) {
        counter.fetch_add(count, Ordering::Relaxed);
    }

    /// Files below the roots selected by the filters, in walk order.
    fn collect_jobs(&self, roots: &[AbsPath]) -> Result<Vec<FileJob>> {
        let fs = self.ctx.require_fs()?;
        let mut options = WalkOptions {
            kind: KindFilter::File,
            globs: self.params.globs.clone(),
            metadata: self.params.sort == SortKey::Modified,
            ..WalkOptions::default()
        };
        self.params.filters.apply(&mut options);
        let walker = Walker::new(options)?;
        let mut jobs = Vec::new();
        let summary = walker.run(fs, roots, &|| self.ctx.checkpoint(), |item| {
            if let WalkItem::Entry(entry) = item
                && (self.params.types.is_empty()
                    || self.params.types.accepts(&entry.name.to_string_lossy()))
            {
                jobs.push(FileJob {
                    modified_ns: entry.meta.as_ref().map_or(0, |meta| meta.modified_ns),
                    path: entry.path,
                });
            }
            ControlFlow::Continue(())
        })?;
        Self::add(&self.totals.denied, summary.permission_denied);
        Ok(jobs)
    }

    /// Searches files, in parallel when the limits allow it; the outcomes
    /// are in the order of `jobs`.
    fn scan_jobs(&self, jobs: &[FileJob]) -> Vec<Result<FileOutcome>> {
        let threads = usize::try_from(self.ctx.limits().threads)
            .unwrap_or(1)
            .clamp(1, jobs.len().max(1));
        if threads == 1 {
            return jobs.iter().map(|job| self.scan_file(job)).collect();
        }
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<Result<FileOutcome>>>> =
            jobs.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let (Some(job), Some(slot)) = (jobs.get(index), slots.get(index)) else {
                            break;
                        };
                        let outcome = self.scan_file(job);
                        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
                    }
                });
            }
        });
        slots
            .into_iter()
            .map(|slot| {
                slot.into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .unwrap_or_else(|| {
                        Err(Error::new(
                            ErrorCode::StageFailed,
                            format!("a thread of `{COMMAND}` ended without a result"),
                        ))
                    })
            })
            .collect()
    }

    fn scan_file(&self, job: &FileJob) -> Result<FileOutcome> {
        self.ctx.checkpoint()?;
        let fs = self.ctx.require_fs()?;
        let meta = match fs.metadata(&job.path, Follow::NoFollow) {
            Ok(meta) => meta,
            Err(error) => return skipped(error),
        };
        let mut handle = match fs.open_read(&job.path) {
            Ok(handle) => handle,
            Err(error) => return skipped(error),
        };
        let mut owned = Vec::new();
        if handle.as_bytes().is_none() {
            if meta.size > self.ctx.limits().max_file_bytes_text {
                return self.scan_stream(job, meta.etag(), handle);
            }
            handle
                .read_to_end(&mut owned)
                .map_err(|error| read_error(&job.path, &error))?;
        }
        let bytes = handle.as_bytes().unwrap_or(&owned);
        let Some(scan) = self.scan_bytes(&job.path, bytes)? else {
            return Ok(FileOutcome::Binary);
        };
        let etag = self.result_etag(&job.path, meta.etag(), Some(bytes), &scan);
        Ok(FileOutcome::Searched(FileResult {
            path: job.path.clone(),
            etag,
            modified_ns: meta.modified_ns,
            scan,
        }))
    }

    /// Searches the content of a file; `None` for a binary file that is
    /// skipped.
    fn scan_bytes(&self, path: &AbsPath, bytes: &[u8]) -> Result<Option<Scan>> {
        let detection = text_codec(self.ctx).detect(bytes);
        if detection.binary && !self.params.binary_text {
            return Ok(None);
        }
        let encoding = detection
            .encoding
            .filter(|encoding| encoding.label != "utf-8" || encoding.bom);
        let decoded = encoding.as_ref().and_then(|encoding| {
            Some((
                decode(bytes, encoding).ok()?,
                OffsetMapper::for_file(encoding)?,
            ))
        });
        let scan = match &decoded {
            Some((text, mapper)) => {
                let mut place = Placement::decoded(text, mapper.clone(), 0);
                self.scan(path, text.as_bytes(), &mut place)?
            }
            None => self.scan(path, bytes, &mut Placement::shifted(0, 0))?,
        };
        Ok(Some(scan))
    }

    /// Searches a large file that the file system does not map as a stream
    /// of UTF-8 bytes.
    fn scan_stream(
        &self,
        job: &FileJob,
        etag: nurt_contracts::fs::Etag,
        mut handle: Box<dyn ReadHandle>,
    ) -> Result<FileOutcome> {
        let Matcher::Text { searcher, engines } = &self.matcher else {
            return Err(too_large(&job.path));
        };
        if self.params.scope != Scope::Any || self.params.enclosing {
            return Err(too_large(&job.path));
        }
        let mut prefix = Vec::new();
        handle
            .by_ref()
            .take(byte_count(STREAM_PREFIX))
            .read_to_end(&mut prefix)
            .map_err(|error| read_error(&job.path, &error))?;
        if text_codec(self.ctx).detect(&prefix).binary && !self.params.binary_text {
            return Ok(FileOutcome::Binary);
        }
        let mut scan = Scan::new(self.pattern_count());
        let mut place = Placement::shifted(0, 0);
        let mut view = None;
        let summary = searcher
            .search_reader(io::Cursor::new(prefix).chain(handle), |hit| {
                self.collect_hit(searcher, engines, hit, &mut view, &mut place, &mut scan)
            })
            .map_err(|error| {
                Error::new(
                    error.code(),
                    format!("searching `{}` failed: {error}", job.path),
                )
            })?;
        scan.finish(&summary);
        let etag = self.result_etag(&job.path, etag, None, &scan);
        Ok(FileOutcome::Searched(FileResult {
            path: job.path.clone(),
            etag,
            modified_ns: job.modified_ns,
            scan,
        }))
    }

    /// Entity tag of a searched file, computed only when records carry it.
    fn result_etag(
        &self,
        path: &AbsPath,
        etag: nurt_contracts::fs::Etag,
        bytes: Option<&[u8]>,
        scan: &Scan,
    ) -> String {
        let carried =
            matches!(self.params.output, Output::Matches | Output::Counts) && !scan.hits.is_empty();
        if carried {
            record_etag(self.ctx, path, etag, bytes)
        } else {
            String::new()
        }
    }

    fn scan(&self, path: &AbsPath, haystack: &[u8], place: &mut Placement<'_>) -> Result<Scan> {
        match &self.matcher {
            Matcher::Text { searcher, engines } => {
                Ok(self.scan_text(searcher, engines, path, haystack, place))
            }
            Matcher::Syntax(syntax) => self.scan_syntax(syntax, path, haystack, place),
        }
    }

    fn code_view<'c>(&'c self, path: &AbsPath, bytes: &'c [u8]) -> Option<CodeView<'c>> {
        if self.params.scope == Scope::Any && !self.params.enclosing {
            return None;
        }
        let service = self.ctx.code()?;
        let language = self
            .params
            .language
            .clone()
            .or_else(|| service.language_of(Some(path.as_path()), bytes));
        let spans = match &language {
            Some(language) if self.params.scope != Scope::Any => {
                service.classify_spans(bytes, language).ok()
            }
            _ => None,
        };
        Some(CodeView {
            service,
            bytes,
            language,
            spans,
            scope: self.params.scope,
            enclosing: self.params.enclosing,
        })
    }

    fn scan_text(
        &self,
        searcher: &Searcher,
        engines: &[Option<RegexEngine>],
        path: &AbsPath,
        haystack: &[u8],
        place: &mut Placement<'_>,
    ) -> Scan {
        let mut view = self.code_view(path, haystack);
        let mut scan = Scan::new(self.pattern_count());
        let summary = searcher.search_slice(haystack, |hit| {
            self.collect_hit(searcher, engines, hit, &mut view, place, &mut scan)
        });
        scan.finish(&summary);
        scan
    }

    fn collect_hit(
        &self,
        searcher: &Searcher,
        engines: &[Option<RegexEngine>],
        hit: &Hit<'_>,
        view: &mut Option<CodeView<'_>>,
        place: &mut Placement<'_>,
        scan: &mut Scan,
    ) -> ControlFlow<()> {
        let view = view.as_ref();
        let start = place.offset(hit.start());
        let mut matches = Vec::new();
        for found in hit.matches() {
            if view.is_none_or(|view| view.accepts(found.start)) {
                matches.push(self.found_match(searcher, engines, hit, found, view, place));
            }
        }
        if matches.is_empty() && !self.params.invert {
            return ControlFlow::Continue(());
        }
        let end = place.offset(hit.end());
        scan.push(Found {
            line: hit.line() + place.line_base,
            start,
            end,
            text: hit.text().into_owned(),
            before: hit
                .before()
                .iter()
                .map(|line| line.text().into_owned())
                .collect(),
            after: hit
                .after()
                .iter()
                .map(|line| line.text().into_owned())
                .collect(),
            matches,
        });
        self.limit_flow(scan)
    }

    /// Stops a search whose matches are filtered by scope once
    /// `max_per_file` is reached; the searcher applies the limit otherwise.
    fn limit_flow(&self, scan: &Scan) -> ControlFlow<()> {
        match self.params.max_per_file {
            Some(limit)
                if self.params.filters_matches()
                    && scan.limited_units(self.params.only_matching) >= limit =>
            {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn found_match(
        &self,
        searcher: &Searcher,
        engines: &[Option<RegexEngine>],
        hit: &Hit<'_>,
        found: &text_search::Match,
        view: Option<&CodeView<'_>>,
        place: &mut Placement<'_>,
    ) -> FoundMatch {
        let regex = engines.get(found.pattern).copied().flatten();
        let (groups, named) = if regex.is_some() && self.params.groups {
            captures(searcher, hit, found)
        } else {
            (None, None)
        };
        FoundMatch {
            pattern: found.pattern,
            start: place.offset(found.start),
            end: place.offset(found.end),
            line: found.line + place.line_base,
            column: found.column,
            end_line: found.end_line + place.line_base,
            end_column: found.end_column,
            text: hit
                .matched(found)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default(),
            engine: regex,
            groups,
            named,
            container: view.and_then(|view| view.container(found.start, place.line_base)),
        }
    }

    fn scan_syntax(
        &self,
        syntax: &SyntaxSearch,
        path: &AbsPath,
        haystack: &[u8],
        place: &mut Placement<'_>,
    ) -> Result<Scan> {
        let mut scan = Scan::new(self.pattern_count());
        let Some(service) = self.ctx.code() else {
            return Ok(scan);
        };
        let detected = service.language_of(Some(path.as_path()), haystack);
        let language = match (&self.params.language, detected) {
            (Some(given), Some(found)) if *given != found => return Ok(scan),
            (Some(given), _) => given.clone(),
            (None, Some(found)) => found,
            (None, None) => return Ok(scan),
        };
        let patterns = syntax.compiled(service, &language, self.params.language.is_some())?;
        let mut found: Vec<(usize, SyntaxMatch)> = Vec::new();
        for (index, pattern) in patterns.iter().enumerate() {
            found.extend(
                service
                    .syntax_search(haystack, pattern.as_ref())?
                    .into_iter()
                    .map(|m| (index, m)),
            );
        }
        found.sort_by_key(|(index, m)| (m.span.bytes.start, *index));
        let table = LineTable::new(haystack);
        let view = self.code_view(path, haystack);
        for (index, syntax_match) in found {
            if self
                .params
                .max_per_file
                .is_some_and(|limit| byte_count(scan.hits.len()) >= limit)
            {
                break;
            }
            if view
                .as_ref()
                .is_none_or(|view| view.accepts(syntax_match.span.bytes.start))
            {
                let hit = self.syntax_hit(
                    index,
                    &syntax_match,
                    &table,
                    view.as_ref(),
                    place,
                    &mut scan.truncated,
                );
                scan.push(hit);
            }
        }
        Ok(scan)
    }

    fn syntax_hit(
        &self,
        pattern: usize,
        found: &SyntaxMatch,
        table: &LineTable<'_>,
        view: Option<&CodeView<'_>>,
        place: &mut Placement<'_>,
        truncated: &mut u64,
    ) -> Found {
        let line = u64::from(found.span.start.line);
        let end_line = u64::from(found.span.end.line);
        let mut render = |no: u64| {
            let (text, cut) = shorten(table.line(no), self.params.max_line);
            *truncated += u64::from(cut);
            text
        };
        let text = render(line);
        let first = line.saturating_sub(byte_count(self.params.before)).max(1);
        let before = (first..line).map(&mut render).collect();
        let last = end_line
            .saturating_add(byte_count(self.params.after))
            .min(table.count());
        let after = (end_line + 1..=last).map(&mut render).collect();
        let start = place.offset(found.span.bytes.start);
        let end = place.offset(found.span.bytes.end);
        let found_match = FoundMatch {
            pattern,
            start,
            end,
            line: line + place.line_base,
            column: u64::from(found.span.start.column),
            end_line: end_line + place.line_base,
            end_column: u64::from(found.span.end.column),
            text: found.text.clone(),
            named: Some(
                found
                    .captures
                    .iter()
                    .map(|capture| (capture.name.clone(), Some(capture.text.clone())))
                    .collect(),
            )
            .filter(|named: &Vec<_>| !named.is_empty()),
            container: view
                .and_then(|view| view.container(found.span.bytes.start, place.line_base)),
            ..FoundMatch::default()
        };
        Found {
            line: line + place.line_base,
            start,
            end,
            text,
            before,
            after,
            matches: vec![found_match],
        }
    }

    /// Searches the text of an input `content` record.
    fn scan_content(&self, record: Record) -> Result<Option<FileResult>> {
        let content = Content::try_from(record).map_err(Error::from)?;
        if content.binary == Some(true) && !self.params.binary_text {
            Self::add(&self.totals.binary, 1);
            return Ok(None);
        }
        let path = resolve(self.ctx, content.path.as_str())?;
        let line_base = content
            .range
            .and_then(|range| range.start())
            .map_or(0, |start| {
                u64::try_from(start.saturating_sub(1)).unwrap_or(0)
            });
        let mapper = match content.range {
            None => OffsetMapper::for_file(&TextEncoding {
                label: content.encoding.clone(),
                bom: content.bom,
            }),
            Some(_) => None,
        }
        .unwrap_or_else(|| OffsetMapper::shifted(0));
        let mut place = Placement::decoded(&content.text, mapper, line_base);
        let scan = self.scan(&path, content.text.as_bytes(), &mut place)?;
        Ok(Some(FileResult {
            path,
            etag: content.etag,
            modified_ns: 0,
            scan,
        }))
    }

    /// Searches the text of an input `line` record.
    fn scan_line(&self, record: Record) -> Result<FileResult> {
        let line = Line::try_from(record).map_err(Error::from)?;
        let path = resolve(self.ctx, line.path.as_str())?;
        let mut place = Placement::shifted(line.start.unwrap_or(0), line.no.saturating_sub(1));
        let scan = self.scan(&path, line.text.as_bytes(), &mut place)?;
        Ok(FileResult {
            path,
            etag: line.etag,
            modified_ns: 0,
            scan,
        })
    }

    fn record_totals(&self, scan: &Scan) {
        Self::add(&self.totals.truncated, scan.truncated);
        Self::add(&self.totals.backtracked, scan.backtracked);
    }

    /// Takes the outcome of one file into account; the result of a searched
    /// file.
    fn settle(&self, outcome: FileOutcome) -> Option<FileResult> {
        match outcome {
            FileOutcome::Searched(result) => {
                self.record_totals(&result.scan);
                Some(result)
            }
            FileOutcome::Binary => {
                Self::add(&self.totals.binary, 1);
                None
            }
            FileOutcome::Denied => {
                Self::add(&self.totals.denied, 1);
                None
            }
            FileOutcome::Vanished => None,
        }
    }

    fn tidy(&self, text: &str) -> String {
        if self.params.trim {
            text.trim().to_owned()
        } else {
            text.to_owned()
        }
    }

    fn tidy_all(&self, lines: &[String], wanted: usize) -> Option<Vec<String>> {
        (wanted > 0).then(|| lines.iter().map(|line| self.tidy(line)).collect())
    }

    /// Records of one searched file.
    fn records(&self, file: &FileResult, summarized: bool) -> Vec<Record> {
        let total = file.total(self.params.invert);
        match self.params.output {
            Output::Matches if !summarized => self.match_records(file),
            Output::Matches | Output::Counts => self.count_records(file, summarized),
            Output::Files if total > 0 => vec![self.entry(file, total)],
            Output::Unmatched if file.scan.hits.is_empty() => vec![self.entry(file, 0)],
            Output::Files | Output::Unmatched => Vec::new(),
        }
    }

    fn match_records(&self, file: &FileResult) -> Vec<Record> {
        let limit = self.params.max_per_file.unwrap_or(u64::MAX);
        let mut seen = HashSet::new();
        let mut records = Vec::new();
        let mut units = 0_u64;
        for found in &file.scan.hits {
            let shown: Vec<Option<&FoundMatch>> =
                if self.params.only_matching && !found.matches.is_empty() {
                    found.matches.iter().map(Some).collect()
                } else {
                    vec![found.matches.first()]
                };
            for found_match in shown {
                if units >= limit {
                    return records;
                }
                let record = self.match_record(file, found, found_match);
                if !self.params.dedupe || seen.insert(record.line.clone()) {
                    units += 1;
                    records.push(Record::from(record));
                }
            }
        }
        records
    }

    fn match_record(
        &self,
        file: &FileResult,
        found: &Found,
        found_match: Option<&FoundMatch>,
    ) -> Match {
        let line = match found_match {
            Some(found_match) if self.params.only_matching => self.tidy(&found_match.text),
            _ => self.tidy(&found.text),
        };
        let mut record = Match {
            path: path_field(&file.path),
            no: found.line,
            column: 1,
            start: found.start,
            end: found.end,
            text: self.tidy(&found.text),
            line,
            etag: file.etag.clone(),
            before: self.tidy_all(&found.before, self.params.before),
            after: self.tidy_all(&found.after, self.params.after),
            ..Match::default()
        };
        if let Some(found_match) = found_match {
            self.fill_match(&mut record, found_match);
        }
        record
    }

    fn fill_match(&self, record: &mut Match, found_match: &FoundMatch) {
        let spans_lines = self.params.multiline || found_match.end_line != found_match.line;
        record.no = found_match.line;
        record.column = found_match.column;
        record.start = found_match.start;
        record.end = found_match.end;
        record.text.clone_from(&found_match.text);
        record.groups.clone_from(&found_match.groups);
        record.named = found_match
            .named
            .clone()
            .map(|named| named.into_iter().collect());
        record.pattern = self.params.listed.then(|| byte_count(found_match.pattern));
        record.container.clone_from(&found_match.container);
        record.regex_engine = found_match.engine;
        record.end_line = spans_lines.then_some(found_match.end_line);
        record.end_column = spans_lines.then_some(found_match.end_column);
    }

    fn count_records(&self, file: &FileResult, summarized: bool) -> Vec<Record> {
        let counted: Vec<(Option<usize>, u64)> = if self.params.listed && !self.params.invert {
            file.scan
                .per_pattern
                .iter()
                .enumerate()
                .filter(|(_, count)| **count > 0)
                .map(|(index, count)| (Some(index), *count))
                .collect()
        } else {
            vec![(None, file.total(self.params.invert))]
        };
        counted
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .filter_map(|(pattern, count)| {
                let (found, found_match) =
                    file.scan.hits.iter().find_map(|found| match pattern {
                        None => Some((found, found.matches.first())),
                        Some(index) => found
                            .matches
                            .iter()
                            .find(|found_match| found_match.pattern == index)
                            .map(|found_match| (found, Some(found_match))),
                    })?;
                let mut record = Match {
                    before: None,
                    after: None,
                    ..self.match_record(file, found, found_match)
                };
                record.count = Some(count);
                record.summarized = summarized.then_some(true);
                Some(Record::from(record))
            })
            .collect()
    }

    fn entry(&self, file: &FileResult, count: u64) -> Record {
        let meta = self
            .ctx
            .fs()
            .and_then(|fs| fs.metadata(&file.path, Follow::NoFollow).ok());
        let mut entry = meta.map_or_else(
            || bare_entry(&file.path),
            |meta| entry_record(&file.path, &meta),
        );
        entry.match_count = Some(count);
        Record::from(entry)
    }

    /// Number of records a file gives in full, as counted by `adaptive`.
    fn full_cost(&self, file: &FileResult) -> u64 {
        file.scan
            .limited_units(self.params.only_matching)
            .min(self.params.max_per_file.unwrap_or(u64::MAX))
    }

    /// Orders searched files by `sort`; the order of the input is kept for
    /// `path` and among equal keys.
    fn sort(&self, files: &mut [FileResult]) {
        let invert = self.params.invert;
        match self.params.sort {
            SortKey::Path => {}
            SortKey::Modified => files.sort_by_key(|file| std::cmp::Reverse(file.modified_ns)),
            SortKey::Count => files.sort_by_key(|file| std::cmp::Reverse(file.total(invert))),
            SortKey::Rank => files.sort_by(|a, b| {
                b.patterns_matched()
                    .cmp(&a.patterns_matched())
                    .then_with(|| b.total(invert).cmp(&a.total(invert)))
                    .then_with(|| {
                        a.path
                            .as_path()
                            .components()
                            .count()
                            .cmp(&b.path.as_path().components().count())
                    })
            }),
        }
    }

    /// Files shown in full by `adaptive`: every file is first counted with
    /// one record; files with the most matches are then shown in full while
    /// the records fit in the limit.
    fn adaptive_selection(&self, files: &[FileResult], limit: u64) -> Vec<bool> {
        let costs: Vec<u64> = files.iter().map(|file| self.full_cost(file)).collect();
        if costs.iter().sum::<u64>() <= limit {
            return vec![true; files.len()];
        }
        let mut full = vec![false; files.len()];
        let shown = byte_count(costs.iter().filter(|&&cost| cost > 0).count());
        let mut budget = limit.saturating_sub(shown);
        let mut order: Vec<usize> = (0..files.len())
            .filter(|&index| costs.get(index).is_some_and(|&cost| cost > 0))
            .collect();
        order.sort_by_key(|&index| {
            std::cmp::Reverse(
                files
                    .get(index)
                    .map_or(0, |file| file.total(self.params.invert)),
            )
        });
        for index in order {
            let extra = costs.get(index).map_or(0, |cost| cost.saturating_sub(1));
            if extra <= budget {
                budget -= extra;
                if let Some(flag) = full.get_mut(index) {
                    *flag = true;
                }
            }
        }
        full
    }
}

fn bare_entry(path: &AbsPath) -> Entry {
    Entry {
        path: path_field(path),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        kind: EntryType::File,
        ..Entry::default()
    }
}

/// Capture groups by number, the whole match first.
type Groups = Option<Vec<Option<String>>>;
/// Named capture groups in the order of the pattern.
type Named = Option<Vec<(String, Option<String>)>>;

fn captures(searcher: &Searcher, hit: &Hit<'_>, found: &text_search::Match) -> (Groups, Named) {
    let Some(captures) = searcher.captures(hit, found) else {
        return (None, None);
    };
    if captures.len() <= 1 {
        return (None, None);
    }
    let groups = (0..captures.len())
        .map(|index| captures.get(index).map(|group| group.text().into_owned()))
        .collect();
    let named: Vec<(String, Option<String>)> = captures
        .named()
        .map(|(name, group)| {
            (
                name.to_owned(),
                group.map(|group| group.text().into_owned()),
            )
        })
        .collect();
    (Some(groups), (!named.is_empty()).then_some(named))
}

fn skipped(error: FsError) -> Result<FileOutcome> {
    match error.kind {
        FsErrorKind::NotFound => Ok(FileOutcome::Vanished),
        FsErrorKind::PermissionDenied => Ok(FileOutcome::Denied),
        _ => Err(Error::from(error)),
    }
}

fn read_error(path: &AbsPath, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
}

fn too_large(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "`{path}` is larger than the text limit, so `{SCOPE}`, `{ENCLOSING}` and syntax patterns cannot be used: search it with a text pattern"
        ),
    )
}

type ResultIter<'a> = Box<dyn Iterator<Item = Result<FileResult>> + Send + 'a>;

/// Results of the files of the source form, searched in batches.
struct FileScanner<'a> {
    run: Arc<Run<'a>>,
    jobs: Vec<FileJob>,
    next: usize,
    ready: VecDeque<Result<FileResult>>,
}

impl<'a> FileScanner<'a> {
    fn new(run: Arc<Run<'a>>, jobs: Vec<FileJob>) -> Self {
        Self {
            run,
            jobs,
            next: 0,
            ready: VecDeque::new(),
        }
    }
}

impl Iterator for FileScanner<'_> {
    type Item = Result<FileResult>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.ready.is_empty() && self.next < self.jobs.len() {
            let threads = usize::try_from(self.run.ctx.limits().threads)
                .unwrap_or(1)
                .max(1);
            let end = (self.next + threads * BATCH_PER_THREAD).min(self.jobs.len());
            let batch = self.jobs.get(self.next..end).unwrap_or_default();
            self.next = end;
            for outcome in self.run.scan_jobs(batch) {
                match outcome {
                    Ok(outcome) => self.ready.extend(self.run.settle(outcome).map(Ok)),
                    Err(error) => {
                        self.ready.push_back(Err(error));
                        self.next = self.jobs.len();
                        break;
                    }
                }
            }
        }
        self.ready.pop_front()
    }
}

/// Results of the input records of the stage form.
struct InputScanner<'a> {
    run: Arc<Run<'a>>,
    input: RecordStream<'a>,
    files: Option<FileScanner<'a>>,
    ready: VecDeque<FileResult>,
    pending_line: Option<FileResult>,
    done: bool,
}

impl<'a> InputScanner<'a> {
    fn new(run: Arc<Run<'a>>, input: RecordStream<'a>) -> Self {
        Self {
            run,
            input,
            files: None,
            ready: VecDeque::new(),
            pending_line: None,
            done: false,
        }
    }

    /// Takes one input record. The lines of one file are joined until a
    /// record of another file or kind arrives.
    fn take(&mut self, record: Record) -> Result<()> {
        if record.kind != RecordKind::Line {
            self.ready.extend(self.pending_line.take());
        }
        match record.kind {
            RecordKind::Entry => {
                let path = record_path(&record, self.run.ctx)?;
                let jobs = self.run.collect_jobs(std::slice::from_ref(&path))?;
                self.files = Some(FileScanner::new(Arc::clone(&self.run), jobs));
            }
            RecordKind::Content => {
                if let Some(result) = self.run.scan_content(record)? {
                    self.run.record_totals(&result.scan);
                    self.ready.push_back(result);
                }
            }
            RecordKind::Line => {
                let result = self.run.scan_line(record)?;
                self.run.record_totals(&result.scan);
                self.merge_line(result);
            }
            other => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`{COMMAND}` takes `entry`, `content` or `line` records, not `{other}`"
                    ),
                ));
            }
        }
        Ok(())
    }

    fn merge_line(&mut self, result: FileResult) {
        match &mut self.pending_line {
            Some(pending) if pending.path == result.path => pending.scan.merge(result.scan),
            _ => self.ready.extend(self.pending_line.replace(result)),
        }
    }
}

impl Iterator for InputScanner<'_> {
    type Item = Result<FileResult>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(result) = self.ready.pop_front() {
                return Some(Ok(result));
            }
            if let Some(item) = self.files.as_mut().and_then(Iterator::next) {
                return Some(item);
            }
            self.files = None;
            if self.done {
                return None;
            }
            let Some(item) = self.input.next() else {
                self.done = true;
                self.ready.extend(self.pending_line.take());
                continue;
            };
            if let Err(error) = item.and_then(|record| self.take(record)) {
                self.done = true;
                self.ready.clear();
                self.pending_line = None;
                return Some(Err(error));
            }
        }
    }
}

/// Records of the results, in the order of `sort`.
struct Emitter<'a> {
    run: Arc<Run<'a>>,
    results: ResultIter<'a>,
    pending: VecDeque<Record>,
    prepared: bool,
    done: bool,
}

impl<'a> Emitter<'a> {
    fn new(run: Arc<Run<'a>>, results: ResultIter<'a>) -> Self {
        Self {
            run,
            results,
            pending: VecDeque::new(),
            prepared: false,
            done: false,
        }
    }

    /// Whether every file is searched before the first record.
    fn collects_all(&self) -> bool {
        let params = &self.run.params;
        params.sort != SortKey::Path
            || (params.adaptive
                && params.output == Output::Matches
                && self.run.ctx.demand().limit.is_some())
    }

    fn prepare_all(&mut self) -> Result<()> {
        self.prepared = true;
        let max = self.run.ctx.limits().max_records;
        let mut files = Vec::new();
        let mut held = 0_u64;
        for result in self.results.by_ref() {
            let result = result?;
            held += byte_count(result.scan.hits.len());
            if held > max {
                return Err(Error::new(
                    ErrorCode::Limit,
                    format!(
                        "`{COMMAND}` holds more than {max} hits for `{SORT}`: narrow the search or sort by path"
                    ),
                ));
            }
            files.push(result);
        }
        self.run.sort(&mut files);
        let full = match self.run.ctx.demand().limit {
            Some(limit) if self.run.params.adaptive => self.run.adaptive_selection(&files, limit),
            _ => vec![true; files.len()],
        };
        for (file, full) in files.iter().zip(full) {
            self.pending.extend(self.run.records(file, !full));
        }
        self.done = true;
        Ok(())
    }
}

impl Iterator for Emitter<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.prepared
            && self.collects_all()
            && let Err(error) = self.prepare_all()
        {
            self.done = true;
            return Some(Err(error));
        }
        loop {
            if let Some(record) = self.pending.pop_front() {
                return Some(Ok(record));
            }
            if self.done {
                return None;
            }
            match self.results.next() {
                None => self.done = true,
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(error));
                }
                Some(Ok(file)) => self.pending.extend(self.run.records(&file, false)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::ByteRange;

    use super::*;

    fn file(path: &str, matches: u64, hits: usize) -> FileResult {
        let found = Found {
            matches: vec![FoundMatch::default()],
            ..Found::default()
        };
        FileResult {
            path: nurt_testkit::vfs::test_path(path).unwrap(),
            etag: String::new(),
            modified_ns: 0,
            scan: Scan {
                hits: vec![found; hits],
                per_pattern: vec![matches],
                matches,
                ..Scan::default()
            },
        }
    }

    fn run_with(output: Output, adaptive: bool, ctx: &ExecCtx) -> Run<'_> {
        let args = BoundArgs::new(COMMAND).with(
            PATTERN,
            nurt_contracts::stage::ArgValue::Value(Value::Text("x".to_owned())),
            false,
        );
        let mut params = Params::read(&args, ctx).unwrap();
        params.output = output;
        params.adaptive = adaptive;
        let matcher = Matcher::new(&params).unwrap();
        Run::new(params, matcher, ctx)
    }

    #[test]
    fn tolerant_literals_become_regular_expressions() {
        assert_eq!(tolerant_regex("a b", Whitespace::Exact), "a b");
        assert_eq!(
            tolerant_regex("x = 1  ", Whitespace::Trailing),
            "x = 1[ \\t]*"
        );
        assert_eq!(
            tolerant_regex("  call()", Whitespace::Indent),
            "[ \\t]*call\\(\\)"
        );
        assert_eq!(
            tolerant_regex(" a   b ", Whitespace::Normalize),
            "[ \\t]*a[ \\t]+b[ \\t]*"
        );
        assert_eq!(
            tolerant_regex("a\r\nb", Whitespace::Indent),
            "[ \\t]*a\\r?\\n[ \\t]*b"
        );
    }

    #[test]
    fn escaping_covers_every_special_character() {
        assert_eq!(escape("a.b*c"), "a\\.b\\*c");
        assert_eq!(
            escape("[x]{1}^$|?+()#&-~\\"),
            "\\[x\\]\\{1\\}\\^\\$\\|\\?\\+\\(\\)\\#\\&\\-\\~\\\\"
        );
        assert_eq!(escape("\u{17c}\u{f3}\u{142}w"), "\u{17c}\u{f3}\u{142}w");
    }

    #[test]
    fn line_table_returns_lines_without_endings() {
        let table = LineTable::new(b"one\r\ntwo\nthree\n");
        assert_eq!(table.count(), 3);
        assert_eq!(table.line(1), b"one");
        assert_eq!(table.line(2), b"two");
        assert_eq!(table.line(3), b"three");
        assert_eq!(table.line(4), b"");
        assert_eq!(LineTable::new(b"").count(), 0);
        assert_eq!(LineTable::new(b"last").line(1), b"last");
    }

    #[test]
    fn long_lines_are_shortened_with_a_marker() {
        assert_eq!(shorten(b"abcdef", 0), ("abcdef".to_owned(), false));
        assert_eq!(shorten(b"abcdef", 6), ("abcdef".to_owned(), false));
        assert_eq!(
            shorten(b"abcdef", 4),
            ("abcd [2 characters omitted]".to_owned(), true)
        );
    }

    #[test]
    fn spans_classify_offsets() {
        let spans = [
            ClassifiedSpan {
                bytes: ByteRange::new(2, 5),
                class: SpanClass::Comment,
            },
            ClassifiedSpan {
                bytes: ByteRange::new(8, 10),
                class: SpanClass::String,
            },
        ];
        assert_eq!(class_at(&spans, 0), SpanClass::Code);
        assert_eq!(class_at(&spans, 2), SpanClass::Comment);
        assert_eq!(class_at(&spans, 5), SpanClass::Code);
        assert_eq!(class_at(&spans, 9), SpanClass::String);
        assert_eq!(class_at(&spans, 10), SpanClass::Code);
    }

    #[test]
    fn adaptive_selection_shows_the_largest_files_that_fit() {
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let run = run_with(Output::Matches, true, &ctx);
        let files = [file("/a", 2, 2), file("/b", 6, 6), file("/c", 3, 3)];
        assert_eq!(run.adaptive_selection(&files, 20), [true, true, true]);
        assert_eq!(run.adaptive_selection(&files, 8), [false, true, false]);
        assert_eq!(run.adaptive_selection(&files, 5), [false, false, true]);
        assert_eq!(run.adaptive_selection(&files, 2), [false, false, false]);
    }

    #[test]
    fn sorting_by_count_and_rank_is_stable() {
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let mut run = run_with(Output::Matches, false, &ctx);
        let names = |files: &[FileResult]| -> Vec<AbsPath> {
            files.iter().map(|file| file.path.clone()).collect()
        };
        let abs = |text: &str| nurt_testkit::vfs::test_path(text).unwrap();
        let mut files = vec![file("/a", 1, 1), file("/b", 3, 3), file("/c", 3, 3)];
        run.params.sort = SortKey::Count;
        run.sort(&mut files);
        assert_eq!(names(&files), [abs("/b"), abs("/c"), abs("/a")]);
        let mut ranked = vec![file("/x/y/deep", 2, 2), file("/top", 2, 2)];
        run.params.sort = SortKey::Rank;
        run.sort(&mut ranked);
        assert_eq!(names(&ranked), [abs("/top"), abs("/x/y/deep")]);
    }

    #[test]
    fn unmatched_output_stops_at_the_first_match() {
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let args = BoundArgs::new(COMMAND)
            .with(
                PATTERN,
                nurt_contracts::stage::ArgValue::Value(Value::Text("x".to_owned())),
                false,
            )
            .with(
                OUTPUT,
                nurt_contracts::stage::ArgValue::Value(Value::Enum("unmatched".to_owned())),
                false,
            );
        let params = Params::read(&args, &ctx).unwrap();
        assert_eq!(params.options, SearchOptions::new().max_per_file(Some(1)));
    }

    #[test]
    fn unmatched_output_with_a_scope_does_not_stop_the_searcher_at_the_first_hit() {
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let args = BoundArgs::new(COMMAND)
            .with(
                PATTERN,
                nurt_contracts::stage::ArgValue::Value(Value::Text("x".to_owned())),
                false,
            )
            .with(
                OUTPUT,
                nurt_contracts::stage::ArgValue::Value(Value::Enum("unmatched".to_owned())),
                false,
            );
        let mut params = Params::read(&args, &ctx).unwrap();
        params.scope = Scope::Comments;
        assert_eq!(params.search_options(true, false), SearchOptions::new());
    }

    #[test]
    fn content_records_with_an_extreme_range_start_do_not_overflow() {
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let args = BoundArgs::new(COMMAND).with(
            PATTERN,
            nurt_contracts::stage::ArgValue::Value(Value::Text("x".to_owned())),
            false,
        );
        let params = Params::read(&args, &ctx).unwrap();
        let matcher = Matcher::new(&params).unwrap();
        let run = Run::new(params, matcher, &ctx);
        let path = nurt_testkit::vfs::test_path("/data/a.txt").unwrap();
        let content = Content {
            path: path_field(&path),
            text: "x\n".to_owned(),
            encoding: "utf-8".to_owned(),
            range: Some(nurt_contracts::value::Range::from_start(i64::MIN)),
            ..Content::default()
        };
        let result = run.scan_content(Record::from(content)).unwrap().unwrap();
        assert_eq!(result.scan.hits.len(), 1);
        assert_eq!(result.scan.hits[0].line, 1);
    }

    #[test]
    fn legacy_encoded_files_report_the_byte_offsets_of_every_hit() {
        let test = nurt_testkit::ctx::TestContext::new();
        let path = nurt_testkit::vfs::test_path("/data/legacy.txt").unwrap();
        test.fs()
            .write_file(
                &path,
                b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\nkoniec\r\n",
            )
            .unwrap();
        let args = BoundArgs::new(COMMAND)
            .with(
                PATTERN,
                nurt_contracts::stage::ArgValue::Value(Value::Text("koniec".to_owned())),
                false,
            )
            .with(
                PATH,
                nurt_contracts::stage::ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            );
        let ctx = test.ctx();
        let output = Search.execute(&args, StageInput::none(), &ctx).unwrap();
        let nurt_contracts::stage::StageData::Records(stream) = output.data else {
            panic!("records expected");
        };
        let found: Vec<Match> = stream
            .map(|item| Match::try_from(item.unwrap()).unwrap())
            .collect();
        let places: Vec<(u64, u64, u64, u64)> = found
            .iter()
            .map(|found| (found.no, found.column, found.start, found.end))
            .collect();
        assert_eq!(places, [(2, 1, 32, 38), (3, 1, 40, 46)]);
    }

    #[test]
    fn long_lines_with_multibyte_characters_are_shortened_by_characters() {
        assert_eq!(
            shorten("\u{17c}\u{f3}\u{142}w".as_bytes(), 2),
            ("\u{17c}\u{f3} [2 characters omitted]".to_owned(), true)
        );
        assert_eq!(
            shorten("\u{17c}\u{f3}".as_bytes(), 2),
            ("\u{17c}\u{f3}".to_owned(), false)
        );
    }

    #[test]
    fn negative_amounts_and_invalid_pattern_lists_are_rejected() {
        let negative = BoundArgs::new(COMMAND).with(
            CONTEXT,
            nurt_contracts::stage::ArgValue::Value(Value::Int(-1)),
            false,
        );
        assert_eq!(
            amount(&negative, CONTEXT).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let test = nurt_testkit::ctx::TestContext::new();
        let ctx = test.ctx();
        let empty = BoundArgs::new(COMMAND).with(
            PATTERN,
            nurt_contracts::stage::ArgValue::Value(Value::List(Vec::new())),
            false,
        );
        assert_eq!(
            Params::read(&empty, &ctx).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let wrong = BoundArgs::new(COMMAND).with(
            PATTERN,
            nurt_contracts::stage::ArgValue::Value(Value::Int(1)),
            false,
        );
        assert_eq!(
            Params::read(&wrong, &ctx).unwrap_err().code(),
            ErrorCode::Type
        );
    }
}
