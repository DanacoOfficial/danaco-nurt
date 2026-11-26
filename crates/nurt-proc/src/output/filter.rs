// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Line cleaning, output patterns, error lines and collapsing of repeated
//! lines.

use std::borrow::Cow;
use std::sync::OnceLock;

use nurt_contracts::error::{Error, ErrorCode, Result};

/// Backtracking steps allowed for one match of a pattern that needs
/// look-around or back-references.
const BACKTRACK_LIMIT: usize = 1_000_000;

/// Built-in error line patterns; the format is described in the file.
const ERROR_PATTERNS: &str = include_str!("error_patterns.txt");

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const BOM: char = '\u{feff}';

/// Removes terminal control sequences, a byte order mark, carriage return
/// redraws and trailing white space from one output line.
///
/// A line redrawn with carriage returns keeps only its last non-empty part,
/// which is what a terminal shows.
pub(super) fn clean_line(line: &str) -> String {
    let stripped = strip_ansi(line);
    let text = stripped.strip_prefix(BOM).unwrap_or(&stripped);
    let text = if text.contains('\r') {
        text.split('\r')
            .rfind(|part| !part.is_empty())
            .unwrap_or_default()
    } else {
        text
    };
    text.trim_end().to_owned()
}

fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.as_bytes().contains(&ESC) {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.bytes().position(|byte| byte == ESC) {
        out.push_str(&rest[..start]);
        let sequence = &rest[start..];
        match escape_length(sequence.as_bytes()) {
            0 => {
                out.push(char::from(ESC));
                rest = &sequence[1..];
            }
            length => rest = &sequence[length..],
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Length of the control sequence at the start of `bytes`, which starts with
/// ESC; 0 when the bytes do not form one.
///
/// Recognised are control sequences (ECMA-48 CSI), operating system commands
/// ended by BEL, ST or the end of the line, character set selections and two
/// byte escape sequences. Every returned length ends on a character boundary
/// because each recognised sequence ends with an ASCII byte or at the end.
fn escape_length(bytes: &[u8]) -> usize {
    match bytes.get(1) {
        Some(b'[') => csi_length(bytes),
        Some(b']') => osc_length(bytes),
        Some(b'(' | b')') => match bytes.get(2) {
            Some(byte) if byte.is_ascii_alphanumeric() => 3,
            _ => 0,
        },
        Some(b'@'..=b'Z' | b'\\'..=b'_' | b'=' | b'>') => 2,
        _ => 0,
    }
}

fn csi_length(bytes: &[u8]) -> usize {
    let parameters = bytes[2..]
        .iter()
        .take_while(|byte| (0x30..=0x3f).contains(*byte))
        .count();
    let intermediates = bytes[2 + parameters..]
        .iter()
        .take_while(|byte| (0x20..=0x2f).contains(*byte))
        .count();
    let end = 2 + parameters + intermediates;
    match bytes.get(end) {
        Some(0x40..=0x7e) => end + 1,
        _ => 0,
    }
}

fn osc_length(bytes: &[u8]) -> usize {
    let body = bytes[2..]
        .iter()
        .take_while(|byte| **byte != BEL && **byte != ESC)
        .count();
    let end = 2 + body;
    match (bytes.get(end), bytes.get(end + 1)) {
        (Some(&BEL), _) => end + 1,
        (Some(&ESC), Some(b'\\')) => end + 2,
        _ => end,
    }
}

/// Regular expression of an output parameter.
///
/// Patterns the linear-time `regex` engine accepts run there; patterns with
/// look-around or back-references run on the backtracking engine with a step
/// limit, and a match that exceeds the limit counts as no match.
#[derive(Debug)]
pub(super) enum Pattern {
    /// Pattern of the linear-time engine.
    Linear(regex::Regex),
    /// Pattern of the backtracking engine.
    Backtracking(fancy_regex::Regex),
}

impl Pattern {
    /// Compiles the pattern given in `parameter`.
    ///
    /// # Errors
    ///
    /// `E_REGEX` when neither engine accepts the pattern.
    pub(super) fn compile(parameter: &str, source: &str) -> Result<Self> {
        if let Ok(linear) = regex::Regex::new(source) {
            return Ok(Self::Linear(linear));
        }
        fancy_regex::RegexBuilder::new(source)
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .map(Self::Backtracking)
            .map_err(|error| {
                Error::new(
                    ErrorCode::Regex,
                    format!("{parameter}: invalid regular expression: {error}"),
                )
                .with_found(source)
            })
    }

    /// Whether the pattern matches somewhere in `text`.
    pub(super) fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Linear(regex) => regex.is_match(text),
            Self::Backtracking(regex) => regex.is_match(text).unwrap_or(false),
        }
    }

    /// The first capture group of a match that holds a number; `None` when
    /// the pattern does not match, `Some(None)` when it matches without a
    /// numeric group.
    pub(super) fn number(&self, text: &str) -> Option<Option<f64>> {
        match self {
            Self::Linear(regex) => regex.captures(text).map(|captures| {
                captures
                    .iter()
                    .skip(1)
                    .flatten()
                    .find_map(|group| parse_number(group.as_str()))
            }),
            Self::Backtracking(regex) => regex.captures(text).ok().flatten().map(|captures| {
                captures
                    .iter()
                    .skip(1)
                    .flatten()
                    .find_map(|group| parse_number(group.as_str()))
            }),
        }
    }
}

fn parse_number(text: &str) -> Option<f64> {
    text.trim()
        .trim_end_matches('%')
        .trim_end()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Built-in recognition of error lines.
#[derive(Debug)]
pub(super) struct ErrorLines {
    error: regex::Regex,
    not_error: Option<regex::Regex>,
}

impl ErrorLines {
    /// The built-in patterns.
    ///
    /// # Errors
    ///
    /// `E_INTERNAL` when the built-in pattern file does not compile.
    pub(super) fn builtin() -> Result<&'static Self> {
        static BUILTIN: OnceLock<std::result::Result<ErrorLines, String>> = OnceLock::new();
        BUILTIN
            .get_or_init(|| Self::parse(ERROR_PATTERNS))
            .as_ref()
            .map_err(|detail| {
                Error::new(
                    ErrorCode::Internal,
                    format!("built-in error line patterns do not compile: {detail}"),
                )
            })
    }

    fn parse(source: &str) -> std::result::Result<Self, String> {
        let mut error = Vec::new();
        let mut not_error = Vec::new();
        for line in source.lines().map(str::trim) {
            if let Some(pattern) = line.strip_prefix('+') {
                error.push(pattern.trim());
            } else if let Some(pattern) = line.strip_prefix('-') {
                not_error.push(pattern.trim());
            } else if !line.is_empty() && !line.starts_with('#') {
                return Err(format!("line without + or -: {line}"));
            }
        }
        if error.is_empty() {
            return Err("no error line pattern".to_owned());
        }
        Ok(Self {
            error: alternation(&error)?,
            not_error: if not_error.is_empty() {
                None
            } else {
                Some(alternation(&not_error)?)
            },
        })
    }

    /// Whether `line` reports an error.
    pub(super) fn is_error(&self, line: &str) -> bool {
        self.error.is_match(line)
            && !self
                .not_error
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(line))
    }
}

fn alternation(patterns: &[&str]) -> std::result::Result<regex::Regex, String> {
    let joined = patterns
        .iter()
        .map(|pattern| format!("(?:{pattern})"))
        .collect::<Vec<_>>()
        .join("|");
    regex::Regex::new(&format!("(?i){joined}")).map_err(|error| error.to_string())
}

/// First words of lines that report one step of a long job, such as
/// `Compiling serde v1.0.1`; consecutive lines starting with the same word
/// are collapsed even when the names and versions after it differ.
const STEP_WORDS: &[&str] = &[
    "Compiling",
    "Checking",
    "Fresh",
    "Downloading",
    "Downloaded",
    "Documenting",
    "Updating",
    "Installing",
    "Collecting",
    "Building",
    "Unpacking",
    "Extracting",
    "Fetching",
    "Linking",
    "Packaging",
    "Locking",
    "Resolving",
    "Preparing",
    "Get:",
    "Hit:",
    "Ign:",
];

/// A line that stands for `raw` consecutive lines of the output.
pub(super) type Collapsed = (String, u64);

/// Collapses runs of consecutive non-empty lines that are identical, or that
/// differ only in numbers, versions and the names after a step word, into
/// the first line of the run followed by a space, the multiplication sign
/// U+00D7 and the count in parentheses, for example `ok (` U+00D7 `3)`.
///
/// Lines that report an error are collapsed only when identical, so that
/// distinct errors stay visible.
#[derive(Debug, Default)]
pub(super) struct Dedupe {
    enabled: bool,
    errors: Option<&'static ErrorLines>,
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    text: String,
    pattern: Option<String>,
    count: u64,
}

impl Dedupe {
    /// Collapsing is done only when `enabled`; otherwise lines pass unchanged.
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            errors: None,
            pending: None,
        }
    }

    /// Keeps lines that match the error patterns out of the collapsing of
    /// similar lines.
    pub(super) fn with_errors(mut self, errors: &'static ErrorLines) -> Self {
        self.errors = Some(errors);
        self
    }

    /// Takes the next line and returns the line that is complete with the
    /// number of lines it stands for, if any.
    pub(super) fn push(&mut self, text: String) -> Option<Collapsed> {
        if !self.enabled {
            return Some((text, 1));
        }
        let pattern = self.pattern(&text);
        if let Some(pending) = &mut self.pending
            && !text.is_empty()
            && (pending.text == text || (pattern.is_some() && pending.pattern == pattern))
        {
            pending.count += 1;
            return None;
        }
        self.pending
            .replace(Pending {
                text,
                pattern,
                count: 1,
            })
            .map(Pending::collapse)
    }

    /// Returns the last held line.
    pub(super) fn finish(&mut self) -> Option<Collapsed> {
        self.pending.take().map(Pending::collapse)
    }

    fn pattern(&self, text: &str) -> Option<String> {
        if self.errors.is_some_and(|errors| errors.is_error(text)) {
            return None;
        }
        similarity_pattern(text)
    }
}

impl Pending {
    fn collapse(self) -> Collapsed {
        (collapsed(self.text, self.count), self.count)
    }
}

/// Form of a line in which similar lines are equal: the step word of a
/// progress line, or the line with every number replaced by `#`. `None` for
/// lines without a step word or a digit.
fn similarity_pattern(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let word = trimmed.split_whitespace().next()?;
    if STEP_WORDS.contains(&word) {
        return Some(format!("step {word}"));
    }
    if !text.bytes().any(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut pattern = String::with_capacity(text.len());
    let mut in_number = false;
    for character in text.chars() {
        if character.is_ascii_digit() {
            if !in_number {
                pattern.push('#');
            }
            in_number = true;
        } else {
            in_number = false;
            pattern.push(character);
        }
    }
    pattern.chars().any(char::is_alphabetic).then_some(pattern)
}

fn collapsed(line: String, count: u64) -> String {
    if count > 1 {
        format!("{line} (\u{d7}{count})")
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_colour_sequences_and_keeps_text() {
        assert_eq!(clean_line("\u{1b}[31merror\u{1b}[0m: x"), "error: x");
        assert_eq!(clean_line("\u{1b}]0;title\u{7}rest"), "rest");
        assert_eq!(clean_line("\u{1b}]8;;http://a\u{1b}\\link"), "link");
        assert_eq!(clean_line("\u{1b}(Bplain"), "plain");
        assert_eq!(clean_line("\u{1b}=keypad"), "keypad");
    }

    #[test]
    fn keeps_escape_that_starts_no_sequence() {
        assert_eq!(clean_line("a\u{1b}[\u{1b}"), "a\u{1b}[\u{1b}");
        assert_eq!(clean_line("a\u{1b}(\u{e9}"), "a\u{1b}(\u{e9}");
        assert_eq!(clean_line("\u{1b}[1;2"), "\u{1b}[1;2");
    }

    #[test]
    fn keeps_last_part_of_carriage_return_redraw() {
        assert_eq!(clean_line("10%\r50%\r100%"), "100%");
        assert_eq!(clean_line("done\r"), "done");
        assert_eq!(clean_line("\r\r"), "");
        assert_eq!(clean_line("\u{feff}first  "), "first");
    }

    #[test]
    fn compiles_look_ahead_on_backtracking_engine() {
        let pattern = Pattern::compile("filter", r"foo(?=bar)").unwrap();
        assert!(matches!(pattern, Pattern::Backtracking(_)));
        assert!(pattern.is_match("foobar"));
        assert!(!pattern.is_match("foobaz"));
    }

    #[test]
    fn rejects_invalid_pattern_with_regex_code() {
        let error = Pattern::compile("filter", "a(").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
    }

    #[test]
    fn reads_first_numeric_group() {
        let pattern = Pattern::compile("progress", r"(\w+) (\d+)%").unwrap();
        assert_eq!(pattern.number("step 45% done"), Some(Some(45.0)));
        assert_eq!(pattern.number("nothing"), None);
        let words = Pattern::compile("progress", r"(\w+) done").unwrap();
        assert_eq!(words.number("all done"), Some(None));
        let fancy = Pattern::compile("progress", r"(?<=at )(\d+\.5)").unwrap();
        assert_eq!(fancy.number("at 12.5"), Some(Some(12.5)));
    }

    #[test]
    fn recognises_error_lines_and_zero_counters() {
        let lines = ErrorLines::builtin().unwrap();
        for line in [
            "error[E0308]: mismatched types",
            "FAILED tests/a.py::test_b",
            "thread 'main' panicked",
            "Traceback (most recent call last):",
            "fatal: not a git repository",
            "warning: unused variable",
            "sh: foo: command not found",
            "\u{2717} check",
        ] {
            assert!(lines.is_error(line), "{line}");
        }
        for line in [
            "test result: ok. 3 passed; 0 failed",
            "all good",
            "0 errors",
        ] {
            assert!(!lines.is_error(line), "{line}");
        }
    }

    #[test]
    fn rejects_pattern_file_without_markers() {
        assert!(ErrorLines::parse("error").is_err());
        assert!(ErrorLines::parse("# only comments").is_err());
        assert!(ErrorLines::parse("+ (").is_err());
    }

    fn collapse_all(mut dedupe: Dedupe, lines: &[&str]) -> Vec<Collapsed> {
        let mut out = Vec::new();
        for line in lines {
            out.extend(dedupe.push((*line).to_owned()));
        }
        out.extend(dedupe.finish());
        out
    }

    fn texts(collapsed: &[Collapsed]) -> Vec<&str> {
        collapsed.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn collapses_repeated_lines_with_counter() {
        let out = collapse_all(Dedupe::new(true), &["a", "a", "a", "", "", "b"]);
        assert_eq!(texts(&out), ["a (\u{d7}3)", "", "", "b"]);
        assert_eq!(
            out.iter().map(|(_, raw)| *raw).collect::<Vec<_>>(),
            [3, 1, 1, 1]
        );
    }

    #[test]
    fn passes_lines_when_collapsing_is_off() {
        let mut dedupe = Dedupe::new(false);
        assert_eq!(dedupe.push("a".to_owned()), Some(("a".to_owned(), 1)));
        assert_eq!(dedupe.push("a".to_owned()), Some(("a".to_owned(), 1)));
        assert_eq!(dedupe.finish(), None);
    }

    #[test]
    fn collapses_lines_that_differ_only_in_numbers_and_versions() {
        let out = collapse_all(
            Dedupe::new(true),
            &[
                "test 1 ok in 0.3s",
                "test 2 ok in 1.7s",
                "test 3 ok in 0.1s",
                "done",
            ],
        );
        assert_eq!(texts(&out), ["test 1 ok in 0.3s (\u{d7}3)", "done"]);
    }

    #[test]
    fn collapses_step_lines_with_different_names() {
        let out = collapse_all(
            Dedupe::new(true),
            &[
                "   Compiling serde v1.0.200",
                "   Compiling tokio v1.38.0 (/src/tokio)",
                "   Compiling regex-syntax v0.8.4",
                "    Finished `dev` profile",
            ],
        );
        assert_eq!(
            texts(&out),
            [
                "   Compiling serde v1.0.200 (\u{d7}3)",
                "    Finished `dev` profile"
            ]
        );
    }

    #[test]
    fn does_not_collapse_bare_counters_or_distinct_lines_without_numbers() {
        let out = collapse_all(Dedupe::new(true), &["1", "2", "3", "alpha", "beta"]);
        assert_eq!(texts(&out), ["1", "2", "3", "alpha", "beta"]);
    }

    #[test]
    fn collapses_error_lines_only_when_identical() {
        let errors = ErrorLines::builtin().unwrap();
        let out = collapse_all(
            Dedupe::new(true).with_errors(errors),
            &[
                "error: file 1 failed",
                "error: file 2 failed",
                "error: file 2 failed",
            ],
        );
        assert_eq!(
            texts(&out),
            ["error: file 1 failed", "error: file 2 failed (\u{d7}2)"]
        );
    }
}
