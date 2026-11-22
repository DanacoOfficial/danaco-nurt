// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Literal and regular expression search in text.
//!
//! A [`Searcher`] holds one or more [`Pattern`]s and the [`SearchOptions`]
//! of a search. It searches a byte slice or a reader and passes every hit,
//! a line or a block of lines with its matches and context, to a closure.
//!
//! # Dialect
//!
//! Regular expressions use the syntax of the Rust `regex` crate, which runs
//! in linear time: groups `( )`, `(?:…)`, named groups `(?<name>…)` and
//! `(?P<name>…)`, classes such as `[a-z]`, `\d`, `\w`, `\s`, `\b`, greedy
//! and lazy quantifiers and Unicode properties `\p{…}`. Classes, `\w`, `\b`
//! and case folding follow Unicode. A pattern with look-ahead,
//! look-behind or a back-reference is evaluated by a backtracking engine
//! instead; its matches report [`RegexEngine::Backtracking`] and every
//! search of one line, or of the whole text with `multiline`, is limited to
//! [`SearchOptions::backtrack_limit`] steps.
//!
//! `^` and `$` match at the start and end of every line, also before
//! `\r\n`. Without `multiline` a match never crosses a line break and each
//! line is a separate text for look-around, `\A` and `\z`.
//!
//! # Positions
//!
//! Byte offsets count from the start of the searched input. Lines count
//! from 1. Columns count Unicode scalar values from 1; an invalid UTF-8
//! sequence counts as one character. Input that is not UTF-8 is searched as
//! bytes: the caller converts other encodings and decides whether binary
//! input is searched at all.
//!
//! # Errors
//!
//! [`SearchError::code`] maps an error to the code of the language:
//! `E_REGEX` for an invalid pattern, `E_LIMIT` for a pattern above the size
//! limit, `E_IO` for a failed read.
//!
//! # Examples
//!
//! ```
//! use std::ops::ControlFlow;
//!
//! use nurt_text::search::{Pattern, SearchOptions, Searcher};
//!
//! let searcher = Searcher::new(&[Pattern::regex(r"fn (?<name>\w+)")], SearchOptions::new())?;
//! let mut found = Vec::new();
//! let summary = searcher.search_slice(b"fn main() {}\nlet x = 1;\nfn helper() {}\n", |hit| {
//!     for m in hit.matches() {
//!         let name = searcher.captures(hit, m).and_then(|c| c.name("name").map(|g| g.text().into_owned()));
//!         found.push((m.line, m.column, name));
//!     }
//!     ControlFlow::Continue(())
//! });
//! assert_eq!(summary.matches, 2);
//! assert_eq!(found[1], (3, 1, Some("helper".to_owned())));
//! # Ok::<(), nurt_text::search::SearchError>(())
//! ```

mod emit;
mod error;
mod hit;
mod pattern;
mod reader;
mod scan;
mod slice;

#[cfg(test)]
mod tests;

use std::io;
use std::ops::ControlFlow;

use nurt_contracts::record::RegexEngine;

pub use self::error::SearchError;
pub use self::hit::{Captures, ContextLine, Group, Hit, Match, Summary};
pub use self::pattern::{DEFAULT_BACKTRACK_LIMIT, Pattern};

use self::pattern::{CompileSettings, PatternSet};
use self::scan::{line_end, line_start};

/// Default number of characters after which a line is shortened.
pub const DEFAULT_MAX_LINE: usize = 500;

/// Options of a search.
///
/// The defaults are those of the `search` command: case-sensitive, no
/// context, lines shortened after [`DEFAULT_MAX_LINE`] characters, no limit
/// of hits.
///
/// # Examples
///
/// ```
/// use nurt_text::search::SearchOptions;
///
/// let options = SearchOptions::new().case_sensitive(false).whole_word(true).context(2).max_per_file(Some(10));
/// # let _ = options;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    case_sensitive: bool,
    whole_word: bool,
    multiline: bool,
    invert: bool,
    only_matching: bool,
    before: usize,
    after: usize,
    max_line: Option<usize>,
    max_per_file: Option<u64>,
    backtrack_limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            whole_word: false,
            multiline: false,
            invert: false,
            only_matching: false,
            before: 0,
            after: 0,
            max_line: Some(DEFAULT_MAX_LINE),
            max_per_file: None,
            backtrack_limit: DEFAULT_BACKTRACK_LIMIT,
        }
    }
}

impl SearchOptions {
    /// Options with the defaults of the `search` command.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether letter case is significant; `false` folds case by Unicode
    /// rules for every pattern.
    #[must_use]
    pub const fn case_sensitive(mut self, yes: bool) -> Self {
        self.case_sensitive = yes;
        self
    }

    /// Whether a match must not be preceded or followed by a word
    /// character (a Unicode letter, digit, mark or connector punctuation).
    #[must_use]
    pub const fn whole_word(mut self, yes: bool) -> Self {
        self.whole_word = yes;
        self
    }

    /// Whether a match may cross line breaks; a hit is then the block of
    /// lines covered by matches that touch each other.
    #[must_use]
    pub const fn multiline(mut self, yes: bool) -> Self {
        self.multiline = yes;
        self
    }

    /// Whether the hits are the lines without a match.
    #[must_use]
    pub const fn invert(mut self, yes: bool) -> Self {
        self.invert = yes;
        self
    }

    /// Whether `max_per_file` counts matches instead of hits; the caller
    /// shows the matched text instead of the line.
    #[must_use]
    pub const fn only_matching(mut self, yes: bool) -> Self {
        self.only_matching = yes;
        self
    }

    /// Number of lines shown before and after each hit.
    #[must_use]
    pub const fn context(self, lines: usize) -> Self {
        self.before(lines).after(lines)
    }

    /// Number of lines shown before each hit.
    #[must_use]
    pub const fn before(mut self, lines: usize) -> Self {
        self.before = lines;
        self
    }

    /// Number of lines shown after each hit.
    #[must_use]
    pub const fn after(mut self, lines: usize) -> Self {
        self.after = lines;
        self
    }

    /// Number of characters after which [`Hit::text`] and
    /// [`ContextLine::text`] shorten a line; 0 keeps every line whole.
    #[must_use]
    pub const fn max_line(mut self, characters: usize) -> Self {
        self.max_line = if characters == 0 {
            None
        } else {
            Some(characters)
        };
        self
    }

    /// Largest number of hits, or of matches with `only_matching`, passed
    /// to the sink; `None` for no limit.
    #[must_use]
    pub const fn max_per_file(mut self, limit: Option<u64>) -> Self {
        self.max_per_file = limit;
        self
    }

    /// Step limit of the backtracking engine for one search of a line, or
    /// of the whole text with `multiline`.
    #[must_use]
    pub const fn backtrack_limit(mut self, steps: usize) -> Self {
        self.backtrack_limit = steps;
        self
    }
}

/// Compiled patterns with the options of a search.
///
/// A searcher is immutable and can search any number of inputs, also from
/// several threads at once.
#[derive(Debug)]
pub struct Searcher {
    set: PatternSet,
    options: SearchOptions,
}

impl Searcher {
    /// Compiles `patterns` for a search with `options`.
    ///
    /// The index of a pattern in `patterns` is [`Match::pattern`]. Where
    /// several patterns match at the same position, the pattern with the
    /// lower index wins.
    ///
    /// # Errors
    ///
    /// [`SearchError::Pattern`] when a pattern is not a valid regular
    /// expression, with the position of the error where the engine reports
    /// it; [`SearchError::TooLarge`] when a compiled pattern exceeds the
    /// size limit.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::record::RegexEngine;
    /// use nurt_text::search::{Pattern, SearchOptions, Searcher};
    ///
    /// let searcher = Searcher::new(&[Pattern::literal("a.b"), Pattern::regex(r"(\w)\1")], SearchOptions::new())?;
    /// assert_eq!(searcher.engine(0), Some(RegexEngine::Linear));
    /// assert_eq!(searcher.engine(1), Some(RegexEngine::Backtracking));
    /// # Ok::<(), nurt_text::search::SearchError>(())
    /// ```
    pub fn new(patterns: &[Pattern], options: SearchOptions) -> Result<Self, SearchError> {
        let settings = CompileSettings {
            case_insensitive: !options.case_sensitive,
            whole_word: options.whole_word,
            backtrack_limit: options.backtrack_limit,
        };
        Ok(Self {
            set: PatternSet::new(patterns, settings)?,
            options,
        })
    }

    /// Options of the search.
    #[must_use]
    pub const fn options(&self) -> &SearchOptions {
        &self.options
    }

    /// Number of patterns.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.set.len()
    }

    /// Engine that evaluates the pattern at `index`.
    #[must_use]
    pub fn engine(&self, index: usize) -> Option<RegexEngine> {
        self.set.get(index).map(|pattern| pattern.engine())
    }

    /// Searches `haystack` and passes each hit to `sink` in the order of
    /// the input; `sink` returns [`ControlFlow::Break`] to stop the search.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::ops::ControlFlow;
    ///
    /// use nurt_text::search::{Pattern, SearchOptions, Searcher};
    ///
    /// let searcher = Searcher::new(&[Pattern::literal("b")], SearchOptions::new().context(1))?;
    /// let mut seen = Vec::new();
    /// searcher.search_slice(b"a\nb\nc\n", |hit| {
    ///     seen.push((hit.before().len(), hit.text().into_owned(), hit.after().len()));
    ///     ControlFlow::Continue(())
    /// });
    /// assert_eq!(seen, [(1, "b".to_owned(), 1)]);
    /// # Ok::<(), nurt_text::search::SearchError>(())
    /// ```
    pub fn search_slice<F>(&self, haystack: &[u8], sink: F) -> Summary
    where
        F: FnMut(&Hit<'_>) -> ControlFlow<()>,
    {
        slice::search(self, haystack, sink)
    }

    /// Searches `reader` and passes each hit to `sink`, with the same
    /// results as [`Searcher::search_slice`] on the bytes read.
    ///
    /// Without `multiline` and without backtracking patterns the reader is
    /// searched line by line with a bounded buffer; otherwise it is read
    /// into memory first.
    ///
    /// # Errors
    ///
    /// [`SearchError::Io`] when reading fails.
    pub fn search_reader<R, F>(&self, mut reader: R, sink: F) -> Result<Summary, SearchError>
    where
        R: io::Read,
        F: FnMut(&Hit<'_>) -> ControlFlow<()>,
    {
        match self.set.line_matcher() {
            Some(matcher) if !self.options.multiline => {
                Ok(reader::search(self, matcher, reader, sink)?)
            }
            _ => {
                let mut haystack = Vec::new();
                reader.read_to_end(&mut haystack)?;
                Ok(self.search_slice(&haystack, sink))
            }
        }
    }

    /// Capture groups of `found`, one of the matches of `hit`; `None` for a
    /// match of another hit.
    ///
    /// Groups are evaluated again on demand, so a search that does not use
    /// them does not pay for them.
    #[must_use]
    pub fn captures<'a>(&'a self, hit: &Hit<'a>, found: &Match) -> Option<Captures<'a>> {
        let pattern = self.set.get(found.pattern)?;
        let (haystack, offset) = hit.haystack();
        let start = usize::try_from(found.start.checked_sub(offset)?).ok()?;
        let (from, to) = if self.options.multiline {
            (0, haystack.len())
        } else {
            (line_start(haystack, start), line_end(haystack, start))
        };
        let (text, base) = if !self.options.multiline && pattern.searches_by_line() {
            (haystack.get(from..to)?, from)
        } else {
            (haystack, 0)
        };
        let spans = pattern.captures(text, start - base, to - base)?;
        let groups = spans
            .into_iter()
            .map(|span| {
                span.and_then(|(s, e)| {
                    Some(hit::Group {
                        start: offset + (base + s) as u64,
                        end: offset + (base + e) as u64,
                        bytes: text.get(s..e)?,
                    })
                })
            })
            .collect();
        Some(Captures {
            groups,
            names: pattern.names(),
        })
    }
}
