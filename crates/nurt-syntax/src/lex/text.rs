// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::ops::Range;

use nurt_contracts::ast::Expr;

use super::error::{Fault, LexError};
use super::scan::{self, Lexeme, Scanned, char_len_at, line_end};
use super::token::{ForeignKind, TokenKind};

/// Maximum number of hex digits of a `\u{…}` escape.
const MAX_UNICODE_ESCAPE_DIGITS: usize = 6;

/// Invalid escape sequence: its byte range and whether it is a `\u` escape.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BadEscape {
    range: Range<usize>,
    unicode: bool,
}

/// Reads the escape sequence at the backslash `at`; returns its length and
/// the character it denotes.
fn read_escape(text: &str, at: usize) -> Result<(usize, char), BadEscape> {
    let simple = |value| Ok((2, value));
    match text.as_bytes().get(at + 1) {
        Some(b'\\') => simple('\\'),
        Some(b'"') => simple('"'),
        Some(b'n') => simple('\n'),
        Some(b'r') => simple('\r'),
        Some(b't') => simple('\t'),
        Some(b'0') => simple('\0'),
        Some(b'u') => read_unicode_escape(text.as_bytes(), at),
        Some(_) => Err(BadEscape {
            range: at..at + 1 + char_len_at(text, at + 1),
            unicode: false,
        }),
        None => Err(BadEscape {
            range: at..at + 1,
            unicode: false,
        }),
    }
}

fn read_unicode_escape(bytes: &[u8], at: usize) -> Result<(usize, char), BadEscape> {
    let invalid = |end: usize| BadEscape {
        range: at..end,
        unicode: true,
    };
    let digits_start = at + 3;
    if bytes.get(at + 2) != Some(&b'{') {
        return Err(invalid(at + 2));
    }
    let digits = bytes
        .get(digits_start..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    let digits_end = digits_start + digits;
    if bytes.get(digits_end) != Some(&b'}') {
        return Err(invalid(digits_end));
    }
    let end = digits_end + 1;
    if digits == 0 || digits > MAX_UNICODE_ESCAPE_DIGITS {
        return Err(invalid(end));
    }
    let value = bytes
        .get(digits_start..digits_end)
        .unwrap_or_default()
        .iter()
        .fold(0_u32, |value, &digit| (value << 4) | hex_value(digit));
    char::from_u32(value)
        .map(|character| (end - at, character))
        .ok_or_else(|| invalid(end))
}

fn hex_value(digit: u8) -> u32 {
    char::from(digit).to_digit(16).unwrap_or_default()
}

/// Scans the text literal whose opening quote is at `start`; reports every
/// invalid escape to `on_bad` and returns the offset after the closing quote,
/// or `None` when the literal is not closed.
fn scan_body(text: &str, start: usize, mut on_bad: impl FnMut(BadEscape)) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start + 1;
    loop {
        let offset = bytes
            .get(index..)?
            .iter()
            .position(|&byte| byte == b'"' || byte == b'\\')?;
        index += offset;
        if bytes.get(index) == Some(&b'"') {
            return Some(index + 1);
        }
        match read_escape(text, index) {
            Ok((len, _)) => index += len,
            Err(bad) => {
                index = bad.range.end;
                on_bad(bad);
            }
        }
    }
}

/// Whether the text literal `literal`, quotes included, has an escape that
/// the language does not have.
pub(crate) fn has_unknown_escape(literal: &str) -> bool {
    let mut found = false;
    scan_body(literal, 0, |bad| found |= !bad.unicode);
    found
}

/// Scans a text literal `"…"` at `start`. With `keep_unknown`, a text whose
/// only invalid escapes are escapes that the language does not have (`\d`,
/// `\q`) is a text token: its value keeps the characters of the escape.
pub(crate) fn scan_text(text: &str, start: usize, keep_unknown: bool) -> Scanned {
    let mut first_bad = None;
    let mut malformed = false;
    let end = scan_body(text, start, |bad| {
        malformed |= bad.unicode;
        first_bad.get_or_insert(bad);
    });
    let Some(end) = end else {
        return Err(unterminated(text, start, "text"));
    };
    match first_bad {
        None => Ok(Lexeme::new(end, TokenKind::Text)),
        Some(_) if keep_unknown && !malformed => Ok(Lexeme::new(end, TokenKind::Text)),
        Some(bad) => Err(scan::reject_with(
            escape_fault(text, start..end, bad),
            Lexeme::new(end, TokenKind::Text),
        )),
    }
}

/// Number of `#` of a raw text that starts at `start`, or `None` when no
/// raw text starts there.
pub(crate) fn raw_hashes(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes().get(start..)?;
    if bytes.first() != Some(&b'r') {
        return None;
    }
    let hashes = bytes
        .iter()
        .skip(1)
        .take_while(|&&byte| byte == b'#')
        .count();
    (bytes.get(1 + hashes) == Some(&b'"')).then_some(hashes)
}

/// Scans the raw text at `start`; [`raw_hashes`] tells whether one starts
/// there.
pub(crate) fn scan_raw(text: &str, start: usize) -> Scanned {
    let hashes = raw_hashes(text, start).unwrap_or_default();
    match raw_close(text, start + hashes + 2, hashes) {
        Some(close) => Ok(Lexeme::new(close + 1 + hashes, TokenKind::RawText)),
        None => Err(unterminated(text, start, "raw text")),
    }
}

/// Offset of the quote that closes a raw text with `hashes` delimiting `#`.
fn raw_close(text: &str, from: usize, hashes: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = from;
    loop {
        let quote = index + bytes.get(index..)?.iter().position(|&byte| byte == b'"')?;
        let delimiter = bytes.get(quote + 1..quote + 1 + hashes);
        if delimiter.is_some_and(|marks| marks.iter().all(|&byte| byte == b'#')) {
            return Some(quote);
        }
        index = quote + 1;
    }
}

fn unterminated(text: &str, start: usize, what: &str) -> Box<scan::Rejected> {
    let first_line_end = line_end(text, start);
    let fault = Fault::lexical(
        start..first_line_end,
        "unterminated_text",
        format!("{what} is not closed; add the closing delimiter"),
        "text",
    )
    .resume_at(text.len());
    scan::reject_with(
        fault,
        Lexeme::new(
            text.len(),
            TokenKind::Foreign(ForeignKind::UnterminatedText { first_line_end }),
        ),
    )
}

fn escape_fault(text: &str, literal: Range<usize>, bad: BadEscape) -> Fault {
    let escape = text.get(bad.range.clone()).unwrap_or_default();
    let content = text
        .get(literal.start + 1..literal.end - 1)
        .unwrap_or_default();
    let fault = if bad.unicode {
        Fault::escape(
            bad.range,
            "invalid_unicode_escape",
            format!(
                "invalid escape `{escape}`; write `\\u{{…}}` with 1 to 6 hex digits of a Unicode scalar value"
            ),
        )
    } else {
        Fault::escape(
            bad.range,
            "unknown_escape",
            format!(
                "unknown escape `{escape}`; use one of \\\\ \\\" \\n \\r \\t \\0 \\u{{…}} or write raw text r\"…\""
            ),
        )
    };
    escape_fixes(fault.resume_at(literal.end), content)
}

/// Adds the replacements of a text with an invalid escape: the raw text, and
/// a path with `/` or a regular expression.
fn escape_fixes(fault: Fault, content: &str) -> Fault {
    let fault = if content.contains("\\\"") {
        fault
    } else {
        fault.with_fix(scan::raw(content))
    };
    if looks_like_path(content) {
        let slashed = content.replace("\\\\", "/").replace('\\', "/");
        fault.with_fix(scan::quote(&slashed))
    } else if content.contains('\n') {
        fault
    } else {
        fault.with_fix(format!("/{}/", content.replace('/', "\\/")))
    }
}

/// Whether a text starts like a Windows path: a drive letter with `:\`, a
/// UNC prefix `\\` or a relative `.\`.
fn looks_like_path(content: &str) -> bool {
    let bytes = content.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2) == Some(&b'\\');
    drive || content.starts_with("\\\\") || content.starts_with(".\\")
}

/// Replaces `\r\n` with `\n` and appends the result to `value`.
fn push_normalized(value: &mut String, mut plain: &str) {
    while let Some(index) = plain.find("\r\n") {
        let (line, rest) = plain.split_at_checked(index).unwrap_or((plain, ""));
        value.push_str(line);
        value.push('\n');
        plain = rest.get(2..).unwrap_or_default();
    }
    value.push_str(plain);
}

/// Value of the body of a text literal with valid escapes.
fn unescape(body: &str) -> String {
    let mut value = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(index) = rest.find('\\') {
        let (plain, tail) = rest.split_at_checked(index).unwrap_or((rest, ""));
        push_normalized(&mut value, plain);
        match read_escape(tail, 0) {
            Ok((len, character)) => {
                value.push(character);
                rest = tail.get(len..).unwrap_or_default();
            }
            Err(bad) => {
                value.push_str(tail.get(bad.range.clone()).unwrap_or_default());
                rest = tail.get(bad.range.end..).unwrap_or_default();
            }
        }
    }
    push_normalized(&mut value, rest);
    value
}

fn not_literal(src: &str, what: &str) -> LexError {
    Fault::lexical(
        0..src.len(),
        "not_a_literal",
        format!("`{src}` is not a single {what} literal"),
        "literals",
    )
    .into_standalone(src)
}

/// Decodes a text literal `"…"`: processes escapes and turns `\r\n` into
/// `\n`.
///
/// # Errors
///
/// Returns `E_ESCAPE` for an invalid escape sequence and `E_LEXICAL` when
/// `src` is not exactly one closed text literal. Positions are relative to
/// `src`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::decode_text;
///
/// let value = decode_text(r#""tab\there \u{1F600}""#)?;
/// assert_eq!(value, Expr::Text { v: "tab\there \u{1F600}".to_owned(), span: None });
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn decode_text(src: &str) -> Result<Expr, LexError> {
    decode(src, false)
}

/// Decodes a text literal like [`decode_text`], except that an escape the
/// language does not have (`\d`) is not an error: the value keeps its
/// characters.
///
/// # Errors
///
/// As [`decode_text`], without `E_ESCAPE` for an unknown escape.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::decode_text_keeping_unknown_escapes;
///
/// let value = decode_text_keeping_unknown_escapes(r#""\d+\t""#)?;
/// assert_eq!(value, Expr::Text { v: "\\d+\t".to_owned(), span: None });
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn decode_text_keeping_unknown_escapes(src: &str) -> Result<Expr, LexError> {
    decode(src, true)
}

fn decode(src: &str, keep_unknown: bool) -> Result<Expr, LexError> {
    if !src.starts_with('"') {
        return Err(not_literal(src, "text"));
    }
    match scan_text(src, 0, keep_unknown) {
        Ok(lexeme) if lexeme.end == src.len() => {
            let body = src.get(1..src.len() - 1).unwrap_or_default();
            Ok(Expr::Text {
                v: unescape(body),
                span: None,
            })
        }
        Ok(_) => Err(not_literal(src, "text")),
        Err(rejected) => Err(rejected.fault.into_standalone(src)),
    }
}

/// Decodes a raw text `r"…"`, `r#"…"#`: the value is the text between the
/// delimiters with `\r\n` turned into `\n`.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one closed raw text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::decode_raw;
///
/// let value = decode_raw(r###"r#"import "fmt""#"###)?;
/// assert_eq!(value, Expr::Text { v: r#"import "fmt""#.to_owned(), span: None });
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn decode_raw(src: &str) -> Result<Expr, LexError> {
    let Some(hashes) = raw_hashes(src, 0) else {
        return Err(not_literal(src, "raw text"));
    };
    match scan_raw(src, 0) {
        Ok(lexeme) if lexeme.end == src.len() => {
            let body = src
                .get(hashes + 2..src.len() - hashes - 1)
                .unwrap_or_default();
            let mut value = String::with_capacity(body.len());
            push_normalized(&mut value, body);
            Ok(Expr::Text {
                v: value,
                span: None,
            })
        }
        Ok(_) => Err(not_literal(src, "raw text")),
        Err(rejected) => Err(rejected.fault.into_standalone(src)),
    }
}

/// Byte ranges, relative to `src`, of the invalid escape sequences of the
/// text literal `src`; empty when `src` is not a closed text literal.
///
/// The tolerant lexer returns a [`TokenKind::Text`] token for a text with
/// invalid escapes; this function locates them.
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::invalid_escapes;
///
/// assert_eq!(invalid_escapes(r#""C:\data\x""#), vec![3..5, 8..10]);
/// ```
#[must_use]
pub fn invalid_escapes(src: &str) -> Vec<Range<usize>> {
    if !src.starts_with('"') {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    match scan_body(src, 0, |bad| ranges.push(bad.range)) {
        Some(_) => ranges,
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_text, invalid_escapes, raw_hashes, unescape};

    #[test]
    fn unescapes_simple_and_unicode_escapes() {
        assert_eq!(
            unescape(r#"a\\b\"c\n\r\t\0\u{e9}"#),
            "a\\b\"c\n\r\t\0\u{e9}"
        );
    }

    #[test]
    fn normalizes_crlf_but_keeps_lone_carriage_return() {
        assert_eq!(unescape("a\r\nb\rc"), "a\nb\rc");
    }

    #[test]
    fn detects_raw_openings() {
        assert_eq!(raw_hashes("r\"x\"", 0), Some(0));
        assert_eq!(raw_hashes("r##\"x\"##", 0), Some(2));
        assert_eq!(raw_hashes("r#x", 0), None);
        assert_eq!(raw_hashes("x", 0), None);
    }

    #[test]
    fn rejects_surrogate_and_out_of_range_escapes() {
        for src in [
            r#""\u{D800}""#,
            r#""\u{110000}""#,
            r#""\u{}""#,
            r#""\u{1234567}""#,
        ] {
            assert!(decode_text(src).is_err(), "{src}");
        }
    }

    #[test]
    fn locates_every_invalid_escape() {
        assert_eq!(invalid_escapes(r#""\d\u{zz}""#), vec![1..3, 3..6]);
        assert!(invalid_escapes(r#""\d"#).is_empty());
    }
}
