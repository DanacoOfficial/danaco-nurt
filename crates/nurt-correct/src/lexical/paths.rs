// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules for literals in path, regular expression and text places.
//!
//! The binder calls these functions once it knows the type of the place of a
//! literal: [`normalize_path`] (`L19`), [`regex_to_path`] (`L22`),
//! [`resolve_relative`] (`L23`) and [`reinterpret_escapes`] (`L24`).
//! [`quote_bare_path`] (`L21`) also runs on the program text before parsing.
//!
//! Path rules change only the notation allowed for paths: the separator `\`
//! becomes `/`, a lower-case drive letter becomes upper case, doubled
//! separators are merged except for the leading `//` of a UNC path and a
//! trailing separator is removed except at a root. The letter case of path
//! segments is never changed.

use std::ops::Range;

use nurt_contracts::ast::{Expr, Span};
use nurt_contracts::error::{At, Error, ErrorCode, Stage};
use nurt_contracts::report::TextPosition;
use nurt_syntax::lex::unknown_escape_texts;

use super::literal::{regex_literal, text_literal};
use super::locate::{Locator, offset_of};
use super::rules::Rule;
use crate::record::{Correction, CorrectionKind};

/// Characters that make an unquoted path or a regular expression read as a
/// pattern rather than as a path.
const PATTERN_CHARACTERS: &str = "*?[](){}+^$|\\";

/// Control characters produced by the escapes `\t`, `\n`, `\r` and `\0` of a
/// text, with the letter of the escape.
const CONTROL_ESCAPES: [(char, char); 4] = [('\t', 't'), ('\n', 'n'), ('\r', 'r'), ('\0', '0')];

/// Literal produced by a path rule, with the record of the correction.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralFix {
    /// Expression that replaces the literal, without a span.
    pub value: Expr,
    /// Record of the correction; its `to` is the canonical notation of
    /// [`LiteralFix::value`].
    pub correction: Correction,
}

impl LiteralFix {
    fn text(rule: Rule, value: String, at: TextPosition, from: &str) -> Self {
        let correction = rule.record(at, from, text_literal(&value), CorrectionKind::Path);
        Self {
            value: Expr::Text {
                v: value,
                span: None,
            },
            correction,
        }
    }
}

/// Place of a text literal with escapes that the language does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapePlace {
    /// Place of type `regex`.
    Regex,
    /// Place of type `path`.
    Path,
    /// Place of type `text` or any other type.
    Text,
}

/// Normalizes the notation of a path value (`L19`, class C).
///
/// The value is the path after escape processing and `at` the position of
/// its literal. Returns `None` when the path is already in canonical notation.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::TextPosition;
/// use nurt_correct::lexical::paths::normalize_path;
///
/// let at = TextPosition { line: 1, column: 6, length: Some(15) };
/// let fix = normalize_path("c:\\proj\\src\\", at).map(|fix| fix.correction.to);
/// assert_eq!(fix.as_deref(), Some("\"C:/proj/src\""));
/// assert!(normalize_path("/srv/App", at).is_none());
/// ```
#[must_use]
pub fn normalize_path(value: &str, at: TextPosition) -> Option<LiteralFix> {
    let normalized = canonical_path(value);
    (normalized != value).then(|| LiteralFix::text(Rule::L19, normalized, at, &text_literal(value)))
}

/// Canonical notation of a path, as described in the module documentation.
fn canonical_path(value: &str) -> String {
    let slashed = value.replace('\\', "/");
    let mut chars: Vec<char> = slashed.chars().collect();
    if let [letter, ':', ..] = chars.as_mut_slice() {
        letter.make_ascii_uppercase();
    }
    let unc = slashed.starts_with("//") && !slashed.starts_with("///");
    let mut out = String::with_capacity(slashed.len());
    for (index, c) in chars.iter().enumerate() {
        let doubled = *c == '/' && out.ends_with('/');
        let keep_unc = unc && index == 1;
        if !doubled || keep_unc {
            out.push(*c);
        }
    }
    while out.ends_with('/') && !is_root(&out) {
        out.pop();
    }
    out
}

/// Whether a path in canonical notation is a root: `/`, `C:/`, `//` or a
/// provider root such as `registry:/`.
fn is_root(path: &str) -> bool {
    if path == "/" || path == "//" {
        return true;
    }
    path.strip_suffix(":/")
        .is_some_and(|prefix| !prefix.is_empty() && !prefix.contains('/'))
}

/// Quotes an unquoted path written in the program (`L21`, class C).
///
/// `written` is the path as written, for example `/srv/app` or `C:\proj`. The
/// text keeps every character of it. Returns `None` when the token reads as
/// a pattern: when it contains `*`, `?`, brackets, `+`, `^`, `$` or `|`, or
/// when it is a single segment such as `/srv`, which may be an unclosed
/// regular expression.
#[must_use]
pub fn quote_bare_path(written: &str, at: TextPosition) -> Option<LiteralFix> {
    if !is_quotable_path(written) {
        return None;
    }
    Some(LiteralFix::text(Rule::L21, written.to_owned(), at, written))
}

fn is_quotable_path(written: &str) -> bool {
    let without_backslashes = written.replace('\\', "/");
    if without_backslashes
        .chars()
        .any(|c| PATTERN_CHARACTERS.contains(c) && c != '\\')
    {
        return false;
    }
    let mut chars = written.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => true,
        (Some('.'), _) => true,
        (Some('/'), _) => {
            let segments = without_backslashes
                .split('/')
                .filter(|segment| !segment.is_empty())
                .count();
            segments >= 2 || without_backslashes.starts_with("//")
        }
        _ => false,
    }
}

/// Turns a regular expression in a path place into a path (`L22`, class S).
///
/// The path is the regular expression as written, including slashes and
/// flags: `list /etc/` gives the path `/etc/` and `read /a/i` the path
/// `/a/i`; rule `L19` then removes a trailing separator. Returns `None` when
/// the expression contains escapes or pattern characters and so does not read
/// as a path.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::TextPosition;
/// use nurt_correct::lexical::paths::regex_to_path;
///
/// let at = TextPosition { line: 1, column: 6, length: Some(5) };
/// let fix = regex_to_path("/etc/", at).map(|fix| fix.correction.to);
/// assert_eq!(fix.as_deref(), Some("\"/etc/\""));
/// assert!(regex_to_path("/a.*b/", at).is_none());
/// ```
#[must_use]
pub fn regex_to_path(written: &str, at: TextPosition) -> Option<LiteralFix> {
    let path_like = written.starts_with('/')
        && written.len() > 2
        && !written.chars().any(|c| PATTERN_CHARACTERS.contains(c));
    path_like.then(|| LiteralFix::text(Rule::L22, written.to_owned(), at, written))
}

/// Resolves a relative path against an explicit base (`L23`, class C).
///
/// The path is in canonical notation, after [`normalize_path`]. The base is
/// the call parameter `base` and `home` the home directory of the account;
/// neither is ever guessed. `.` and `..` segments are resolved lexically. Returns
/// `Ok(None)` for an absolute path.
///
/// # Errors
///
/// `E_PATH_RELATIVE` when the path is relative and no base is given, when it
/// starts with `~` and no home directory is known, or when the base itself is
/// not absolute.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::TextPosition;
/// use nurt_correct::lexical::paths::resolve_relative;
///
/// let at = TextPosition { line: 1, column: 6, length: Some(7) };
/// let fix = resolve_relative("./src", Some("/srv/app"), None, at)?;
/// assert_eq!(fix.map(|fix| fix.correction.to).as_deref(), Some("\"/srv/app/src\""));
/// assert!(resolve_relative("./src", None, None, at).is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn resolve_relative(
    path: &str,
    base: Option<&str>,
    home: Option<&str>,
    at: TextPosition,
) -> Result<Option<LiteralFix>, Error> {
    if is_absolute(path) {
        return Ok(None);
    }
    let (anchor, rest, missing) = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            (home, rest, "the home directory of the account is unknown")
        }
        _ => (
            base,
            path,
            "a relative path needs the call parameter `base`",
        ),
    };
    let Some(anchor) = anchor else {
        return Err(relative_error(path, at, missing));
    };
    if !is_absolute(anchor) {
        return Err(relative_error(
            path,
            at,
            "the call parameter `base` must be an absolute directory",
        ));
    }
    let resolved = join_lexically(&canonical_path(anchor), rest);
    Ok(Some(LiteralFix::text(
        Rule::L23,
        resolved,
        at,
        &text_literal(path),
    )))
}

fn relative_error(path: &str, at: TextPosition, problem: &str) -> Error {
    let error = Error::new(
        ErrorCode::PathRelative,
        format!("relative path `{path}`: {problem}"),
    )
    .with_stage(Stage::Bind)
    .with_at(to_at(at))
    .with_found(path)
    .with_problem(problem)
    .with_help("help corrections");
    match drive_path_from_escapes(path) {
        Some(written) => error
            .with_fix(text_literal(&written))
            .with_fix(text_literal(&canonical_path(&written))),
        None => error,
    }
}

/// The path as it was written when a drive letter is followed by a control
/// character that an escape of a text produced: `C:\temp` read as `C:`, a tab
/// and `emp`. `None` for any other path.
fn drive_path_from_escapes(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let (Some(letter), Some(':'), Some(first)) = (chars.next(), chars.next(), chars.next()) else {
        return None;
    };
    if !letter.is_ascii_alphabetic() || !CONTROL_ESCAPES.iter().any(|(c, _)| *c == first) {
        return None;
    }
    let mut written = String::with_capacity(path.len() + 2);
    for c in path.chars() {
        match CONTROL_ESCAPES.iter().find(|(control, _)| *control == c) {
            Some((_, escape)) => {
                written.push('\\');
                written.push(*escape);
            }
            None => written.push(c),
        }
    }
    Some(written)
}

/// Whether a path is absolute: `/x`, `C:/x`, `//server/share` or a provider
/// path such as `registry:/HKCU`.
fn is_absolute(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    path.split_once(":/").is_some_and(|(prefix, _)| {
        !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Joins a relative path to an absolute base, resolving `.` and `..`.
fn join_lexically(base: &str, relative: &str) -> String {
    let (root, base_rest) = split_root(base);
    let mut segments: Vec<&str> = base_rest.split('/').filter(|s| !s.is_empty()).collect();
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    format!("{root}{}", segments.join("/"))
}

/// Splits an absolute path into its root (`/`, `C:/`, `//`, `registry:/`)
/// and the rest.
fn split_root(path: &str) -> (&str, &str) {
    if let Some(rest) = path.strip_prefix("//") {
        return (&path[..2], rest);
    }
    if let Some(rest) = path.strip_prefix('/') {
        return (&path[..1], rest);
    }
    match path.find(":/") {
        Some(index) => (&path[..index + 2], &path[index + 2..]),
        None => ("", path),
    }
}

/// Reads a text with escapes that change a pattern or a path according to its
/// place (`L24`, class P).
///
/// The literal is passed as written, including quotes, for example
/// `"\d+"` or `"C:\temp"`. In a regular expression place the characters
/// between the quotes become the pattern when an escape is unknown; in a path
/// place they become the path when the text is written as a Windows path
/// (`C:\`, `\\server`, `.\`) and has an unknown escape or one that makes a
/// control character (`\t`, `\n`), so that backslashes read as separators.
/// The characters of the source are kept in both cases. Returns `Ok(None)`
/// when the literal has no escape that needs another reading.
///
/// # Errors
///
/// `E_ESCAPE` for an unknown escape in a text place, and in a path place when
/// the text is not written as a Windows path, with the literal rewritten as
/// raw text in `fix`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::TextPosition;
/// use nurt_correct::lexical::paths::{EscapePlace, reinterpret_escapes};
///
/// let at = TextPosition { line: 1, column: 8, length: Some(5) };
/// let fix = reinterpret_escapes(r#""\d+""#, EscapePlace::Regex, at)?;
/// assert_eq!(fix.map(|fix| fix.correction.to).as_deref(), Some(r"/\d+/"));
/// assert!(reinterpret_escapes(r#""\d+""#, EscapePlace::Text, at).is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn reinterpret_escapes(
    written: &str,
    place: EscapePlace,
    at: TextPosition,
) -> Result<Option<LiteralFix>, Error> {
    let Some(content) = written
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return Ok(None);
    };
    let escapes = Escapes::of(content);
    match place {
        EscapePlace::Regex if escapes.unknown => Ok(Some(regex_fix(content, written, at))),
        EscapePlace::Path
            if looks_like_windows_path(content) && (escapes.unknown || escapes.control) =>
        {
            if escapes.quote {
                return Ok(None);
            }
            Ok(Some(path_fix(content, written, at)))
        }
        EscapePlace::Path | EscapePlace::Text if escapes.unknown => {
            Err(escape_error(content, written, at))
        }
        _ => Ok(None),
    }
}

/// Whether a text starts like a Windows path: a drive letter with `:\`, a UNC
/// prefix `\\` or a relative `.\`.
fn looks_like_windows_path(content: &str) -> bool {
    let bytes = content.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2) == Some(&b'\\');
    drive || content.starts_with("\\\\") || content.starts_with(".\\")
}

/// A text literal of a program that has an escape the language does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapedText {
    /// Position of the literal in the program text.
    pub at: TextPosition,
    /// The literal as written, quotes included.
    pub written: String,
    range: Range<usize>,
}

/// The text literals of a program text that have an escape the language does
/// not have, in the order of the text.
///
/// A program that holds one does not parse strictly (`E_ESCAPE`); the parser
/// that keeps such a text leaves its reading to the place of the literal, see
/// [`reinterpret_escapes`].
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::paths::escaped_texts;
///
/// let texts = escaped_texts(r#"search "\d+" "/srv""#);
/// assert_eq!(texts.len(), 1);
/// assert_eq!(texts[0].written, r#""\d+""#);
/// assert_eq!((texts[0].at.line, texts[0].at.column), (1, 8));
/// ```
#[must_use]
pub fn escaped_texts(source: &str) -> Vec<EscapedText> {
    let locator = Locator::identity(source);
    unknown_escape_texts(source)
        .into_iter()
        .filter_map(|range| {
            Some(EscapedText {
                at: locator.at(range.clone()),
                written: source.get(range.clone())?.to_owned(),
                range,
            })
        })
        .collect()
}

/// The text literal at the span of a node of the tree, as written in the
/// program text the tree was read from; `None` when the span is not a text
/// literal of that text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Span;
/// use nurt_correct::lexical::paths::written_literal;
///
/// let span = Span { line: 1, column: 8, end_line: 1, end_column: 13 };
/// assert_eq!(written_literal(r#"search "\d+" "/srv""#, span), Some(r#""\d+""#));
/// ```
#[must_use]
pub fn written_literal(source: &str, span: Span) -> Option<&str> {
    let start = offset_of(source, span.line, span.column);
    let end = offset_of(source, span.end_line, span.end_column);
    source
        .get(start..end)
        .filter(|text| text.len() >= 2 && text.starts_with('"') && text.ends_with('"'))
}

/// Reads the text literal at the span of a node of the tree according to its
/// place, as [`reinterpret_escapes`] does for a literal as written.
///
/// The error is [`escape_error_in`]: the whole program as a fix.
///
/// # Errors
///
/// `E_ESCAPE`, see [`reinterpret_escapes`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Span;
/// use nurt_correct::lexical::paths::{EscapePlace, read_escapes_at};
///
/// let source = r#"search "\d+" "/srv""#;
/// let span = Span { line: 1, column: 8, end_line: 1, end_column: 13 };
/// assert!(read_escapes_at(source, span, EscapePlace::Regex)?.is_some());
/// let error = read_escapes_at(source, span, EscapePlace::Text).unwrap_err();
/// assert_eq!(error.data().fix, [r#"search r"\d+" "/srv""#]);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn read_escapes_at(
    source: &str,
    span: Span,
    place: EscapePlace,
) -> Result<Option<LiteralFix>, Error> {
    let Some(written) = written_literal(source, span) else {
        return Ok(None);
    };
    let at = TextPosition {
        line: span.line,
        column: span.column,
        length: None,
    };
    match reinterpret_escapes(written, place, at) {
        Err(error) => Err(escaped_texts(source)
            .iter()
            .find(|text| (text.at.line, text.at.column) == (span.line, span.column))
            .map_or(error, |text| escape_error_in(source, text))),
        other => other,
    }
}

/// `E_ESCAPE` for a text literal that no place of the program reads in
/// another way, with the programs that write it as raw text, or as a path
/// with `/` when it is written as a Windows path, in `fix`.
///
/// # Examples
///
/// ```
/// use nurt_correct::lexical::paths::{escape_error_in, escaped_texts};
///
/// let source = r#"write "/srv/a.txt" "a\d" mode: append"#;
/// let texts = escaped_texts(source);
/// let error = escape_error_in(source, &texts[0]);
/// assert_eq!(error.data().fix, [r#"write "/srv/a.txt" r"a\d" mode: append"#]);
/// ```
#[must_use]
pub fn escape_error_in(source: &str, text: &EscapedText) -> Error {
    let content = text
        .written
        .get(1..text.written.len() - 1)
        .unwrap_or_default();
    let program = |literal: &str| {
        format!(
            "{}{literal}{}",
            source.get(..text.range.start).unwrap_or_default(),
            source.get(text.range.end..).unwrap_or_default()
        )
    };
    let raw = text_literal(content);
    let mut error = Error::new(
        ErrorCode::Escape,
        format!(
            "unknown escape in text {}; write it as raw text {raw}",
            text.written
        ),
    )
    .with_stage(Stage::Parse)
    .with_at(to_at(text.at))
    .with_found(text.written.clone())
    .with_problem("unknown_escape")
    .with_help("text");
    if !content.contains("\\\"") {
        error = error.with_fix(program(&raw));
    }
    if looks_like_windows_path(content) {
        let slashed = text_literal(&canonical_path(content));
        error = error.with_fix(program(&slashed));
    }
    error
}

/// Kinds of escapes found in the characters between the quotes of a text.
#[derive(Debug, Default)]
struct Escapes {
    /// An escape that the language does not have, such as `\d` or `\p`.
    unknown: bool,
    /// An escape of a control character: `\n`, `\r`, `\t`, `\0`, `\u{…}`.
    control: bool,
    /// An escaped quote `\"`.
    quote: bool,
}

impl Escapes {
    fn of(content: &str) -> Self {
        let mut escapes = Self::default();
        let mut chars = content.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                continue;
            }
            match chars.next() {
                Some('\\') => {}
                Some('"') => escapes.quote = true,
                Some('n' | 'r' | 't' | '0' | 'u') => escapes.control = true,
                _ => escapes.unknown = true,
            }
        }
        escapes
    }
}

fn regex_fix(content: &str, written: &str, at: TextPosition) -> LiteralFix {
    let pattern = content.replace("\\/", "/");
    let correction = Rule::L24.record(
        at,
        written,
        regex_literal(&pattern, ""),
        CorrectionKind::Regex,
    );
    LiteralFix {
        value: Expr::Regex {
            v: pattern,
            flags: String::new(),
            span: None,
        },
        correction,
    }
}

fn path_fix(content: &str, written: &str, at: TextPosition) -> LiteralFix {
    LiteralFix::text(Rule::L24, content.to_owned(), at, written)
}

fn escape_error(content: &str, written: &str, at: TextPosition) -> Error {
    let raw = text_literal(content);
    Error::new(
        ErrorCode::Escape,
        format!("unknown escape in text {written}; write it as raw text {raw}"),
    )
    .with_stage(Stage::Parse)
    .with_at(to_at(at))
    .with_found(written)
    .with_problem("text has an escape that the language does not have")
    .with_fix(raw)
    .with_help("help corrections")
}

fn to_at(at: TextPosition) -> At {
    At {
        line: Some(at.line),
        column: Some(at.column),
        length: at.length,
        stage: Some(Stage::Parse),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::report::CorrectionClass;

    use super::*;

    const AT: TextPosition = TextPosition {
        line: 1,
        column: 1,
        length: Some(1),
    };

    fn normalized(value: &str) -> Option<String> {
        normalize_path(value, AT).map(|fix| match fix.value {
            Expr::Text { v, .. } => v,
            _ => String::new(),
        })
    }

    #[test]
    fn converts_backslashes_drive_letter_and_trailing_separator() {
        assert_eq!(normalized(r"C:\proj\src\").as_deref(), Some("C:/proj/src"));
        assert_eq!(normalized("c:/Proj").as_deref(), Some("C:/Proj"));
        assert_eq!(normalized("/srv//app///x/").as_deref(), Some("/srv/app/x"));
    }

    #[test]
    fn keeps_roots_unc_prefix_and_segment_case() {
        assert_eq!(normalized("/"), None);
        assert_eq!(normalized("C:/"), None);
        assert_eq!(normalized("c:\\").as_deref(), Some("C:/"));
        assert_eq!(
            normalized(r"\\server\Share\Dir").as_deref(),
            Some("//server/Share/Dir")
        );
        assert_eq!(normalized("/Srv/APP"), None);
        assert_eq!(
            normalized("registry:/HKCU/"),
            Some("registry:/HKCU".to_owned())
        );
    }

    #[test]
    fn path_correction_is_certain_and_reports_both_forms() {
        let fix = normalize_path(r"C:\proj\src\", AT);
        let correction = fix.map(|fix| fix.correction);
        assert_eq!(
            correction.as_ref().map(|c| c.class),
            Some(CorrectionClass::Certain)
        );
        assert_eq!(
            correction.as_ref().map(|c| c.rule.to_string()).as_deref(),
            Some("L19")
        );
        assert_eq!(
            correction.as_ref().map(|c| c.from.as_str()),
            Some(r#"r"C:\proj\src\""#)
        );
    }

    #[test]
    fn quotes_absolute_drive_and_relative_bare_paths() {
        let to = |written| quote_bare_path(written, AT).map(|fix| fix.correction.to);
        assert_eq!(to("/srv/app").as_deref(), Some("\"/srv/app\""));
        assert_eq!(to(r"C:\proj").as_deref(), Some(r#"r"C:\proj""#));
        assert_eq!(to("C:/x").as_deref(), Some("\"C:/x\""));
        assert_eq!(to("./src").as_deref(), Some("\"./src\""));
        assert_eq!(to("//server/share").as_deref(), Some("\"//server/share\""));
    }

    #[test]
    fn leaves_pattern_like_and_single_segment_bare_paths() {
        assert!(quote_bare_path("/srv", AT).is_none());
        assert!(quote_bare_path("/srv/*.log", AT).is_none());
        assert!(quote_bare_path("/a(b/c", AT).is_none());
        assert!(quote_bare_path("srv/app", AT).is_none());
    }

    #[test]
    fn regex_in_path_place_becomes_its_source_text() {
        let to = |written| regex_to_path(written, AT).map(|fix| fix.correction.to);
        assert_eq!(to("/etc/").as_deref(), Some("\"/etc/\""));
        assert_eq!(to("/a/i").as_deref(), Some("\"/a/i\""));
        assert_eq!(to("/x|y/"), None);
        assert_eq!(to(r"/a\/b/"), None);
        assert_eq!(
            regex_to_path("/etc/", AT).map(|fix| fix.correction.class),
            Some(CorrectionClass::Structural)
        );
    }

    #[test]
    fn resolves_relative_paths_only_against_explicit_base() {
        let resolve = |path, base| {
            resolve_relative(path, base, Some("/home/u"), AT)
                .map(|fix| fix.map(|fix| fix.correction.to))
        };
        assert_eq!(
            resolve("./src", Some("/srv/app")).ok().flatten().as_deref(),
            Some("\"/srv/app/src\"")
        );
        assert_eq!(
            resolve("../lib/x", Some("/srv/app/"))
                .ok()
                .flatten()
                .as_deref(),
            Some("\"/srv/lib/x\"")
        );
        assert_eq!(
            resolve("src", Some("C:/proj")).ok().flatten().as_deref(),
            Some("\"C:/proj/src\"")
        );
        assert_eq!(
            resolve("~/notes", None).ok().flatten().as_deref(),
            Some("\"/home/u/notes\"")
        );
        assert_eq!(resolve("/abs", None).ok().flatten(), None);
        let error = resolve("./src", None).err();
        assert_eq!(
            error.map(|error| error.code()),
            Some(ErrorCode::PathRelative)
        );
        let relative_base = resolve("./src", Some("srv")).err();
        assert_eq!(
            relative_base.map(|error| error.code()),
            Some(ErrorCode::PathRelative)
        );
    }

    #[test]
    fn control_characters_after_a_drive_letter_give_the_written_path_as_a_fix() {
        let error = resolve_relative("C:\temp\new.txt", None, None, AT).err();
        assert_eq!(
            error.as_ref().map(Error::code),
            Some(ErrorCode::PathRelative)
        );
        assert_eq!(
            error.map(|error| error.fix.clone()),
            Some(vec![
                r#"r"C:\temp\new.txt""#.to_owned(),
                "\"C:/temp/new.txt\"".to_owned()
            ])
        );
        let drive_only = resolve_relative("C:data", None, None, AT).err();
        assert_eq!(drive_only.map(|error| error.fix.clone()), Some(Vec::new()));
        let later = resolve_relative("a\tb", None, None, AT).err();
        assert_eq!(later.map(|error| error.fix.clone()), Some(Vec::new()));
    }

    #[test]
    fn root_of_a_posix_base_is_the_leading_slash_even_when_a_segment_has_a_colon() {
        assert_eq!(split_root("/mnt/a:/b"), ("/", "mnt/a:/b"));
        assert_eq!(split_root("C:/proj"), ("C:/", "proj"));
        assert_eq!(split_root("//server/share"), ("//", "server/share"));
        assert_eq!(split_root("registry:/HKCU"), ("registry:/", "HKCU"));
        let resolved = resolve_relative("../../c", Some("/mnt/a:/b"), None, AT)
            .ok()
            .flatten()
            .map(|fix| fix.correction.to);
        assert_eq!(resolved.as_deref(), Some("\"/mnt/c\""));
    }

    #[test]
    fn unknown_escape_in_regex_place_becomes_a_regex_with_the_same_characters() {
        let fix = reinterpret_escapes(r#""\d+\.go""#, EscapePlace::Regex, AT)
            .ok()
            .flatten();
        let fix = fix.map(|fix| (fix.value, fix.correction.class));
        assert_eq!(
            fix,
            Some((
                Expr::Regex {
                    v: r"\d+\.go".to_owned(),
                    flags: String::new(),
                    span: None
                },
                CorrectionClass::Probable
            ))
        );
    }

    #[test]
    fn escapes_in_path_place_read_as_separators() {
        let to = |written| {
            reinterpret_escapes(written, EscapePlace::Path, AT)
                .ok()
                .flatten()
                .map(|fix| fix.correction.to)
        };
        assert_eq!(to(r#""C:\temp""#).as_deref(), Some(r#"r"C:\temp""#));
        assert_eq!(to(r#""C:\proj\src""#).as_deref(), Some(r#"r"C:\proj\src""#));
        assert_eq!(to(r#""C:\\proj""#), None);
        assert_eq!(to(r#""a\"b\x""#), None);
    }

    #[test]
    fn unknown_escape_in_text_place_is_an_escape_error_with_raw_fix() {
        let error = reinterpret_escapes(r#""\d+""#, EscapePlace::Text, AT).err();
        assert_eq!(error.as_ref().map(Error::code), Some(ErrorCode::Escape));
        assert_eq!(
            error.map(|error| error.fix.clone()),
            Some(vec![r#"r"\d+""#.to_owned()])
        );
    }

    #[test]
    fn control_escapes_change_only_a_path_written_like_a_windows_path() {
        let to = |written| {
            reinterpret_escapes(written, EscapePlace::Path, AT)
                .ok()
                .flatten()
                .map(|fix| fix.correction.to)
        };
        assert_eq!(
            to(r#""C:\temp\new.txt""#).as_deref(),
            Some(r#"r"C:\temp\new.txt""#)
        );
        assert_eq!(
            to(r#""\\server\tmp""#).as_deref(),
            Some(r#"r"\\server\tmp""#)
        );
        assert_eq!(to(r#"".\new\t""#).as_deref(), Some(r#"r".\new\t""#));
        assert_eq!(to(r#""/srv/a\nb""#), None);
        assert_eq!(to(r#""a\tb""#), None);
    }

    #[test]
    fn unknown_escape_in_a_path_that_is_not_a_windows_path_is_an_escape_error() {
        let error = reinterpret_escapes(r#""src\q""#, EscapePlace::Path, AT).err();
        assert_eq!(error.as_ref().map(Error::code), Some(ErrorCode::Escape));
        assert_eq!(
            error.map(|error| error.fix.clone()),
            Some(vec![r#"r"src\q""#.to_owned()])
        );
    }

    #[test]
    fn finds_the_texts_with_unknown_escapes_and_fixes_their_programs() {
        let source = "write \"/srv/a.txt\" \"a\\d\" mode: append\nlist \"C:\\proj\\q\"";
        let texts = escaped_texts(source);
        let written: Vec<&str> = texts.iter().map(|text| text.written.as_str()).collect();
        assert_eq!(written, ["\"a\\d\"", "\"C:\\proj\\q\""]);
        assert_eq!((texts[1].at.line, texts[1].at.column), (2, 6));
        let first = escape_error_in(source, &texts[0]);
        assert_eq!(first.code(), ErrorCode::Escape);
        assert_eq!(
            first.data().fix,
            ["write \"/srv/a.txt\" r\"a\\d\" mode: append\nlist \"C:\\proj\\q\""]
        );
        let second = escape_error_in(source, &texts[1]);
        assert_eq!(
            second.data().fix,
            [
                "write \"/srv/a.txt\" \"a\\d\" mode: append\nlist r\"C:\\proj\\q\"",
                "write \"/srv/a.txt\" \"a\\d\" mode: append\nlist \"C:/proj/q\""
            ]
        );
    }

    #[test]
    fn reads_the_literal_at_a_span() {
        let source = "list \"a\"\nfind \"é\\d\"";
        let span = |line, column, end_column| Span {
            line,
            column,
            end_line: line,
            end_column,
        };
        assert_eq!(written_literal(source, span(1, 6, 9)), Some("\"a\""));
        assert_eq!(written_literal(source, span(2, 6, 11)), Some("\"é\\d\""));
        assert_eq!(written_literal(source, span(1, 1, 5)), None);
    }

    #[test]
    fn known_escapes_in_regex_or_text_place_need_no_reading() {
        assert_eq!(
            reinterpret_escapes(r#""a\tb""#, EscapePlace::Regex, AT)
                .ok()
                .flatten(),
            None
        );
        assert_eq!(
            reinterpret_escapes(r#""a\nb""#, EscapePlace::Text, AT)
                .ok()
                .flatten(),
            None
        );
        assert_eq!(
            reinterpret_escapes("r\"\\d\"", EscapePlace::Text, AT)
                .ok()
                .flatten(),
            None
        );
    }
}
