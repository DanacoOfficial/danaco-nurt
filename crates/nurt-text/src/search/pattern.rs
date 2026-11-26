// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Compilation of search patterns into matching engines.

use aho_corasick::{AhoCorasick, MatchKind};
use fancy_regex::BytesMode;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use memchr::memmem;
use nurt_contracts::record::RegexEngine;
use nurt_contracts::value::RegexFlags;

use super::error::SearchError;

/// Default limit of steps of the backtracking engine for one search.
pub const DEFAULT_BACKTRACK_LIMIT: usize = 1_000_000;

/// Word boundary written before a pattern when `whole_word` is set: the
/// preceding character, if any, is not a word character.
const WORD_START: &str = r"\b{start-half}";

/// Word boundary written after a pattern when `whole_word` is set: the
/// following character, if any, is not a word character.
const WORD_END: &str = r"\b{end-half}";

/// Line terminator of the line-oriented reader search.
const LINE_TERMINATOR: u8 = b'\n';

/// Search pattern.
///
/// A [`Pattern::Literal`] matches its text character for character; a
/// [`Pattern::Regex`] is a regular expression in the dialect of the
/// language (see the [module documentation](super)).
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::RegexFlags;
/// use nurt_text::search::Pattern;
///
/// let literal = Pattern::literal("a.b");
/// let regex = Pattern::regex(r"\d{4}");
/// let flags: RegexFlags = "i".parse().unwrap();
/// let insensitive = Pattern::regex_with_flags("warning", flags);
/// assert_ne!(literal, regex);
/// assert_ne!(regex, insensitive);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pattern {
    /// Text matched character for character.
    Literal(String),
    /// Regular expression.
    Regex {
        /// Source of the expression, without the enclosing slashes.
        source: String,
        /// Flags written after the closing slash.
        flags: RegexFlags,
    },
}

impl Pattern {
    /// Creates a literal pattern.
    #[must_use]
    pub fn literal(text: impl Into<String>) -> Self {
        Self::Literal(text.into())
    }

    /// Creates a regular expression without flags.
    #[must_use]
    pub fn regex(source: impl Into<String>) -> Self {
        Self::regex_with_flags(source, RegexFlags::default())
    }

    /// Creates a regular expression with flags.
    #[must_use]
    pub fn regex_with_flags(source: impl Into<String>, flags: RegexFlags) -> Self {
        Self::Regex {
            source: source.into(),
            flags,
        }
    }
}

/// Settings shared by every pattern of a searcher.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CompileSettings {
    pub(crate) case_insensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) backtrack_limit: usize,
}

/// Flags written inline into the source of a compiled expression.
#[derive(Debug, Clone, Copy)]
struct InlineFlags {
    case_insensitive: bool,
    dot_all: bool,
    extended: bool,
}

/// The backtracking engine stopped at its step limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StepLimit;

/// Span of a match as byte offsets into the searched slice.
pub(crate) type Span = (usize, usize);

#[derive(Debug)]
enum Engine {
    Literal(memmem::Finder<'static>),
    Linear(regex::bytes::Regex),
    Backtracking(fancy_regex::Regex),
}

/// One pattern compiled for its engine.
#[derive(Debug)]
pub(crate) struct CompiledPattern {
    engine: Engine,
    /// Source for the line filter of the reader search; `None` for patterns
    /// that need the backtracking engine.
    line_source: Option<String>,
    /// Whether the pattern is searched in each line as a separate text
    /// without `multiline`: backtracking patterns, so that look-around stays
    /// within the line, and patterns with `\A` or `\z`, which then anchor to
    /// the line as in the reader search.
    by_line: bool,
    /// Capture group names by group index; index 0 is the whole match.
    names: Vec<Option<String>>,
}

impl CompiledPattern {
    /// Compiles `pattern`, the pattern at `index` in the list of patterns.
    pub(crate) fn new(
        index: usize,
        pattern: &Pattern,
        settings: CompileSettings,
    ) -> Result<Self, SearchError> {
        match pattern {
            Pattern::Literal(text) => compile_literal(index, text, settings),
            Pattern::Regex { source, flags } => compile_regex(index, source, *flags, settings),
        }
    }

    /// Engine reported in the results of this pattern.
    pub(crate) fn engine(&self) -> RegexEngine {
        match self.engine {
            Engine::Literal(_) | Engine::Linear(_) => RegexEngine::Linear,
            Engine::Backtracking(_) => RegexEngine::Backtracking,
        }
    }

    pub(crate) const fn searches_by_line(&self) -> bool {
        self.by_line
    }

    pub(crate) fn names(&self) -> &[Option<String>] {
        &self.names
    }

    fn literal_needle(&self) -> Option<&[u8]> {
        match &self.engine {
            Engine::Literal(finder) => Some(finder.needle()),
            Engine::Linear(_) | Engine::Backtracking(_) => None,
        }
    }

    /// Finds the leftmost match that starts at `from` or later and ends at
    /// `bound` or earlier; text after `bound` is not visible to the engine.
    pub(crate) fn find(
        &self,
        haystack: &[u8],
        from: usize,
        bound: usize,
    ) -> Result<Option<Span>, StepLimit> {
        let Some(visible) = haystack.get(..bound) else {
            return Ok(None);
        };
        if from > bound {
            return Ok(None);
        }
        match &self.engine {
            Engine::Literal(finder) => Ok(visible
                .get(from..)
                .and_then(|rest| finder.find(rest))
                .map(|at| (from + at, from + at + finder.needle().len()))),
            Engine::Linear(regex) => Ok(regex.find_at(visible, from).map(|m| (m.start(), m.end()))),
            Engine::Backtracking(regex) => regex
                .find_from_pos(visible, from)
                .map(|found| found.map(|m| (m.start(), m.end())))
                .map_err(|_| StepLimit),
        }
    }

    /// Spans of the capture groups of the match that starts at `start`, with
    /// the same visibility rule as [`CompiledPattern::find`].
    pub(crate) fn captures(
        &self,
        haystack: &[u8],
        start: usize,
        bound: usize,
    ) -> Option<Vec<Option<Span>>> {
        let visible = haystack.get(..bound)?;
        let groups = match &self.engine {
            Engine::Literal(finder) => {
                let end = start + finder.needle().len();
                (visible.get(start..end)? == finder.needle()).then(|| vec![Some((start, end))])?
            }
            Engine::Linear(regex) => {
                let found = regex.captures_at(visible, start)?;
                (0..found.len())
                    .map(|i| found.get(i).map(|m| (m.start(), m.end())))
                    .collect()
            }
            Engine::Backtracking(regex) => {
                let found = regex.captures_from_pos(visible, start).ok()??;
                (0..found.len())
                    .map(|i| found.get(i).map(|m| (m.start(), m.end())))
                    .collect()
            }
        };
        let whole: &Option<Span> = groups.first()?;
        (whole.map(|(s, _)| s) == Some(start)).then_some(groups)
    }
}

/// Every pattern of a searcher with the engines shared by all of them.
#[derive(Debug)]
pub(crate) struct PatternSet {
    patterns: Vec<CompiledPattern>,
    /// Leftmost-first automaton over all patterns when every pattern is a
    /// non-empty case-sensitive literal.
    literals: Option<AhoCorasick>,
    /// Line filter of the reader search; `None` when some pattern cannot be
    /// expressed for it.
    line_matcher: Option<RegexMatcher>,
    /// Sources searched independently, computed once for every scan.
    sources: Vec<MatchSource>,
}

/// Source of matches within a [`PatternSet`]: one pattern or the automaton
/// over all literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchSource {
    Pattern(usize),
    Literals,
}

impl PatternSet {
    pub(crate) fn new(
        patterns: &[Pattern],
        settings: CompileSettings,
    ) -> Result<Self, SearchError> {
        let patterns = patterns
            .iter()
            .enumerate()
            .map(|(index, pattern)| CompiledPattern::new(index, pattern, settings))
            .collect::<Result<Vec<_>, _>>()?;
        let literals = literal_automaton(&patterns);
        let line_matcher = line_matcher(&patterns);
        let sources = if literals.is_some() {
            vec![MatchSource::Literals]
        } else {
            (0..patterns.len()).map(MatchSource::Pattern).collect()
        };
        Ok(Self {
            patterns,
            literals,
            line_matcher,
            sources,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.patterns.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&CompiledPattern> {
        self.patterns.get(index)
    }

    pub(crate) fn line_matcher(&self) -> Option<&RegexMatcher> {
        self.line_matcher.as_ref()
    }

    /// Sources searched independently and merged by leftmost start.
    pub(crate) fn sources(&self) -> &[MatchSource] {
        &self.sources
    }

    pub(crate) fn searches_by_line(&self, source: MatchSource) -> bool {
        match source {
            MatchSource::Pattern(index) => self
                .get(index)
                .is_some_and(CompiledPattern::searches_by_line),
            MatchSource::Literals => false,
        }
    }

    /// Finds the leftmost match of `source` as `(pattern, start, end)`.
    pub(crate) fn find(
        &self,
        source: MatchSource,
        haystack: &[u8],
        from: usize,
        bound: usize,
    ) -> Result<Option<(usize, usize, usize)>, StepLimit> {
        match source {
            MatchSource::Pattern(index) => {
                let Some(pattern) = self.get(index) else {
                    return Ok(None);
                };
                Ok(pattern
                    .find(haystack, from, bound)?
                    .map(|(s, e)| (index, s, e)))
            }
            MatchSource::Literals => Ok(self.find_literals(haystack, from, bound)),
        }
    }

    fn find_literals(
        &self,
        haystack: &[u8],
        from: usize,
        bound: usize,
    ) -> Option<(usize, usize, usize)> {
        let automaton = self.literals.as_ref()?;
        let visible = haystack.get(..bound)?;
        if from > bound {
            return None;
        }
        let input = aho_corasick::Input::new(visible).span(from..bound);
        automaton
            .find(input)
            .map(|m| (m.pattern().as_usize(), m.start(), m.end()))
    }
}

fn compile_literal(
    index: usize,
    text: &str,
    settings: CompileSettings,
) -> Result<CompiledPattern, SearchError> {
    let flags = InlineFlags {
        case_insensitive: settings.case_insensitive,
        dot_all: false,
        extended: false,
    };
    let source = inline_source(&regex::escape(text), flags, settings.whole_word);
    let engine = if settings.case_insensitive || settings.whole_word {
        Engine::Linear(build_linear(&source).map_err(|error| linear_error(index, None, &error))?)
    } else {
        Engine::Literal(memmem::Finder::new(text.as_bytes()).into_owned())
    };
    Ok(CompiledPattern {
        engine,
        line_source: Some(source),
        by_line: false,
        names: vec![None],
    })
}

fn compile_regex(
    index: usize,
    source: &str,
    flags: RegexFlags,
    settings: CompileSettings,
) -> Result<CompiledPattern, SearchError> {
    let flags = InlineFlags {
        case_insensitive: settings.case_insensitive || flags.case_insensitive,
        dot_all: flags.dot_matches_new_line,
        extended: flags.extended,
    };
    let checked = regex::bytes::RegexBuilder::new(source)
        .case_insensitive(flags.case_insensitive)
        .multi_line(true)
        .crlf(true)
        .dot_matches_new_line(flags.dot_all)
        .ignore_whitespace(flags.extended)
        .build();
    match checked {
        Ok(regex) => {
            let line_source = inline_source(source, flags, settings.whole_word);
            let regex = if settings.whole_word {
                build_linear(&line_source).map_err(|error| linear_error(index, None, &error))?
            } else {
                regex
            };
            let names = regex
                .capture_names()
                .map(|name| name.map(str::to_owned))
                .collect();
            Ok(CompiledPattern {
                engine: Engine::Linear(regex),
                line_source: Some(line_source),
                by_line: source.contains(r"\A") || source.contains(r"\z"),
                names,
            })
        }
        Err(regex::Error::Syntax(text)) => {
            compile_backtracking(index, source, flags, settings, &text)
        }
        Err(error) => Err(linear_error(index, Some(source), &error)),
    }
}

/// Compiles a pattern the linear engine rejected; `linear_message` is the
/// rejection, reported when the backtracking engine rejects the pattern too
/// and the rejection is not about a construct only that engine supports.
fn compile_backtracking(
    index: usize,
    source: &str,
    flags: InlineFlags,
    settings: CompileSettings,
    linear_message: &str,
) -> Result<CompiledPattern, SearchError> {
    let built = fancy_regex::RegexBuilder::new(source)
        .case_insensitive(flags.case_insensitive)
        .multi_line(true)
        .crlf(true)
        .dot_matches_new_line(flags.dot_all)
        .ignore_whitespace(flags.extended)
        .bytes_mode(BytesMode::UnicodeBytes)
        .backtrack_limit(settings.backtrack_limit)
        .build();
    let regex = match built {
        Ok(_) if settings.whole_word => build_backtracking(
            &inline_source(source, flags, true),
            settings.backtrack_limit,
        )
        .map_err(|error| backtracking_error(index, None, &error))?,
        Ok(regex) => regex,
        Err(error) if needs_backtracking(linear_message) => {
            return Err(backtracking_error(index, Some(source), &error));
        }
        Err(_) => return Err(syntax_error(index, source, linear_message)),
    };
    let names = regex
        .capture_names()
        .map(|name| name.map(str::to_owned))
        .collect();
    Ok(CompiledPattern {
        engine: Engine::Backtracking(regex),
        line_source: None,
        by_line: true,
        names,
    })
}

/// Writes `body` with its flags inline, so that the source keeps its meaning
/// when joined with other patterns into the line filter.
///
/// Multi-line mode and CRLF mode are always on: `^` and `$` match at line
/// boundaries and `$` matches before `\r\n`. In extended mode a line break
/// before the closing parenthesis ends a trailing `#` comment of `body`.
fn inline_source(body: &str, flags: InlineFlags, whole_word: bool) -> String {
    let mut source = String::with_capacity(body.len() + WORD_START.len() + WORD_END.len() + 8);
    if whole_word {
        source.push_str(WORD_START);
    }
    source.push_str("(?mR");
    for (set, letter) in [
        (flags.case_insensitive, 'i'),
        (flags.dot_all, 's'),
        (flags.extended, 'x'),
    ] {
        if set {
            source.push(letter);
        }
    }
    source.push(':');
    source.push_str(body);
    if flags.extended {
        source.push('\n');
    }
    source.push(')');
    if whole_word {
        source.push_str(WORD_END);
    }
    source
}

fn build_linear(source: &str) -> Result<regex::bytes::Regex, regex::Error> {
    regex::bytes::RegexBuilder::new(source).build()
}

fn build_backtracking(
    source: &str,
    limit: usize,
) -> Result<fancy_regex::Regex, fancy_regex::Error> {
    fancy_regex::RegexBuilder::new(source)
        .bytes_mode(BytesMode::UnicodeBytes)
        .backtrack_limit(limit)
        .build()
}

fn literal_automaton(patterns: &[CompiledPattern]) -> Option<AhoCorasick> {
    if patterns.len() < 2 {
        return None;
    }
    let needles = patterns
        .iter()
        .map(|pattern| pattern.literal_needle().filter(|needle| !needle.is_empty()))
        .collect::<Option<Vec<_>>>()?;
    AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostFirst)
        .build(needles)
        .ok()
}

/// Builds the line filter; a pattern that contains a line break or a
/// backtracking construct leaves the reader search to the in-memory search.
fn line_matcher(patterns: &[CompiledPattern]) -> Option<RegexMatcher> {
    if patterns.is_empty() {
        return None;
    }
    let sources = patterns
        .iter()
        .map(|pattern| pattern.line_source.as_deref())
        .collect::<Option<Vec<_>>>()?;
    RegexMatcherBuilder::new()
        .line_terminator(Some(LINE_TERMINATOR))
        .build_many(&sources)
        .ok()
}

/// Whether the linear engine rejected a construct that only the
/// backtracking engine supports: look-around or a back-reference.
fn needs_backtracking(linear_message: &str) -> bool {
    linear_message.contains("look-around") || linear_message.contains("backreferences")
}

fn linear_error(index: usize, source: Option<&str>, error: &regex::Error) -> SearchError {
    match (error, source) {
        (regex::Error::CompiledTooBig(limit), _) => SearchError::TooLarge {
            pattern: index,
            limit: *limit,
        },
        (regex::Error::Syntax(text), Some(source)) => syntax_error(index, source, text),
        (other, _) => SearchError::Pattern {
            pattern: index,
            position: None,
            message: lower_first(&other.to_string()),
        },
    }
}

/// Converts the diagnostic of the linear engine into an error with the
/// position of the first marked character.
///
/// The diagnostic quotes a single-line pattern indented by four spaces and
/// marks the offending characters with `^` on the next line; the message
/// follows `error: ` on the last line.
fn syntax_error(index: usize, source: &str, text: &str) -> SearchError {
    const QUOTE_INDENT: usize = 4;
    let message = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("error: "))
        .unwrap_or(text);
    let position = if source.contains('\n') {
        None
    } else {
        text.lines()
            .nth(2)
            .and_then(|marks| marks.find('^'))
            .and_then(|column| column.checked_sub(QUOTE_INDENT))
            .map(|column| column + 1)
    };
    SearchError::Pattern {
        pattern: index,
        position,
        message: lower_first(message),
    }
}

fn backtracking_error(
    index: usize,
    source: Option<&str>,
    error: &fancy_regex::Error,
) -> SearchError {
    match error {
        fancy_regex::Error::ParseError(offset, kind) => SearchError::Pattern {
            pattern: index,
            position: source
                .and_then(|source| source.get(..*offset))
                .map(|prefix| prefix.chars().count() + 1),
            message: lower_first(&kind.to_string()),
        },
        fancy_regex::Error::CompileError(kind) => SearchError::Pattern {
            pattern: index,
            position: None,
            message: lower_first(&kind.to_string()),
        },
        other => SearchError::Pattern {
            pattern: index,
            position: None,
            message: lower_first(&other.to_string()),
        },
    }
}

/// Lower-cases the first character, the form of every message of the
/// program.
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(chars).collect()
    })
}
