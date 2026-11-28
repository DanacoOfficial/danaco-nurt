// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::regex
// Responsibility:   Flags of a regular expression in canonical order.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::ast::Expr;

use super::error::{Fault, LexError};
use super::foreign::bare_path_end;
use super::scan::{self, Lexeme, Scanned, line_end};
use super::token::TokenKind;

/// Flags of a regular expression in canonical order.
const FLAGS: [u8; 4] = *b"imsx";

/// Problems of a fragment after `/` that the tolerant lexer reads as an
/// unquoted path.
const PATH_PROBLEMS: [&str; 3] = ["unterminated_regex", "empty_regex", "unquoted_path"];

/// Whether a regular expression fault with `problem` reads as a path.
pub(crate) fn is_path_problem(problem: &str) -> bool {
    PATH_PROBLEMS.contains(&problem)
}

/// Scans a regular expression `/…/flags` whose opening `/` is at `start`.
pub(crate) fn scan_regex(text: &str, start: usize) -> Scanned {
    let bytes = text.as_bytes();
    let Some(close) = closing_slash(bytes, start) else {
        return Err(scan::reject(unterminated(text, start)));
    };
    let flags_end = start_of_non_word(bytes, close + 1);
    if close == start + 1 {
        return Err(scan::reject(empty(text, start, flags_end)));
    }
    let flags = bytes.get(close + 1..flags_end).unwrap_or_default();
    match check_flags(flags) {
        Ok(()) => Ok(Lexeme::new(flags_end, TokenKind::Regex)),
        Err(problem) => Err(scan::reject(flag_fault(
            text, start, close, flags_end, problem,
        ))),
    }
}

/// Offset of the `/` that closes the pattern opened at `start`. Every `\`
/// pairs with the next character, so `\/` and `\\` never close it.
fn closing_slash(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    loop {
        match bytes.get(index)? {
            b'/' => return Some(index),
            b'\n' => return None,
            b'\\' if bytes.get(index + 1).is_some_and(|&next| next != b'\n') => index += 2,
            _ => index += 1,
        }
    }
}

/// End of the run of ASCII letters, digits and `_` that starts at `from`.
fn start_of_non_word(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .count()
}

/// Problem with the flags of a regular expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlagProblem {
    /// A digit or `_` directly follows the expression.
    Suffix,
    /// A letter outside `imsx`.
    Unknown,
    /// A flag written twice.
    Repeated,
}

fn check_flags(flags: &[u8]) -> Result<(), FlagProblem> {
    if flags.iter().any(|byte| !byte.is_ascii_alphabetic()) {
        return Err(FlagProblem::Suffix);
    }
    if flags.iter().any(|byte| !FLAGS.contains(byte)) {
        return Err(FlagProblem::Unknown);
    }
    let mut seen = [false; FLAGS.len()];
    for flag in flags {
        if let Some(index) = FLAGS.iter().position(|known| known == flag) {
            if seen.get(index).copied().unwrap_or_default() {
                return Err(FlagProblem::Repeated);
            }
            if let Some(slot) = seen.get_mut(index) {
                *slot = true;
            }
        }
    }
    Ok(())
}

/// Valid flags of `flags` in canonical order, each once.
fn canonical_flags(flags: &[u8]) -> String {
    FLAGS
        .iter()
        .filter(|flag| flags.contains(flag))
        .map(|&flag| char::from(flag))
        .collect()
}

/// Whether an expression with invalid flags reads as a path: one of the
/// flag letters is outside `imsx` and `g`, or the path continues after the
/// flags.
fn reads_as_path(flags: &[u8], continues: bool) -> bool {
    continues
        || flags
            .iter()
            .any(|flag| flag.is_ascii_alphabetic() && !FLAGS.contains(flag) && *flag != b'g')
}

fn path_fault(text: &str, start: usize, problem: &'static str, message: String) -> Fault {
    let end = bare_path_end(text, start);
    let path = text.get(start..end).unwrap_or_default();
    Fault::lexical(start..end, problem, message, "regex").with_fix(scan::quote(path))
}

fn unterminated(text: &str, start: usize) -> Fault {
    let end = line_end(text, start);
    let path_end = bare_path_end(text, start);
    let message = "regular expression is not closed; end it with `/` on the same line or write a path as text \"…\"";
    if path_end > start + 1 {
        return path_fault(text, start, "unterminated_regex", message.to_owned()).resume_at(end);
    }
    Fault::lexical(start..end, "unterminated_regex", message, "regex")
}

fn empty(text: &str, start: usize, flags_end: usize) -> Fault {
    let message =
        "empty regular expression; write a pattern between the slashes or a path as text \"…\"";
    if bare_path_end(text, start) > flags_end {
        return path_fault(text, start, "empty_regex", message.to_owned());
    }
    Fault::lexical(start..flags_end, "empty_regex", message, "regex")
}

fn flag_fault(
    text: &str,
    start: usize,
    close: usize,
    flags_end: usize,
    problem: FlagProblem,
) -> Fault {
    let bytes = text.as_bytes();
    let flags = bytes.get(close + 1..flags_end).unwrap_or_default();
    let flags_text = text.get(close + 1..flags_end).unwrap_or_default();
    let continues = bare_path_end(text, start) > flags_end;
    if problem != FlagProblem::Repeated && reads_as_path(flags, continues) {
        let message = format!(
            "`{}` is not a regular expression; write a path as text \"…\"",
            text.get(start..bare_path_end(text, start))
                .unwrap_or_default()
        );
        return path_fault(text, start, "unquoted_path", message);
    }
    let pattern = text.get(start..=close).unwrap_or_default();
    let fixed = format!("{pattern}{}", canonical_flags(flags));
    let (name, message) = match problem {
        FlagProblem::Suffix => (
            "regex_suffix",
            format!(
                "`{flags_text}` directly follows the regular expression; separate it with a space"
            ),
        ),
        FlagProblem::Unknown if flags.contains(&b'g') => (
            "unknown_regex_flag",
            format!(
                "unknown flag in `{flags_text}`; the flags are i, m, s and x, and every match is used without `g`"
            ),
        ),
        FlagProblem::Unknown => (
            "unknown_regex_flag",
            format!("unknown flag in `{flags_text}`; the flags are i, m, s and x"),
        ),
        FlagProblem::Repeated => (
            "repeated_regex_flag",
            format!("flag repeated in `{flags_text}`; write each flag once"),
        ),
    };
    let fault = Fault::lexical(start..flags_end, name, message, "regex");
    if problem == FlagProblem::Suffix {
        fault
    } else {
        fault.with_fix(fixed)
    }
}

/// Decodes a regular expression `/…/flags`: `\/` becomes `/`, every other
/// `\` pair is kept, and the flags are returned in the canonical order
/// `imsx`.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one valid regular
/// expression. Positions are relative to `src`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::decode_regex;
///
/// let value = decode_regex(r"/a\/b\d/si")?;
/// assert_eq!(
///     value,
///     Expr::Regex { v: r"a/b\d".to_owned(), flags: "is".to_owned(), span: None }
/// );
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn decode_regex(src: &str) -> Result<Expr, LexError> {
    if !src.starts_with('/') {
        return Err(not_regex(src));
    }
    let lexeme = scan_regex(src, 0).map_err(|rejected| rejected.fault.into_standalone(src))?;
    if lexeme.end != src.len() {
        return Err(not_regex(src));
    }
    let close = closing_slash(src.as_bytes(), 0).unwrap_or_default();
    let pattern = src.get(1..close).unwrap_or_default();
    let flags = src.as_bytes().get(close + 1..).unwrap_or_default();
    Ok(Expr::Regex {
        v: unescape_slashes(pattern),
        flags: canonical_flags(flags),
        span: None,
    })
}

fn not_regex(src: &str) -> LexError {
    Fault::lexical(
        0..src.len(),
        "not_a_literal",
        format!("`{src}` is not a single regular expression literal"),
        "regex",
    )
    .into_standalone(src)
}

fn unescape_slashes(pattern: &str) -> String {
    let mut value = String::with_capacity(pattern.len());
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            value.push(character);
            continue;
        }
        match characters.next() {
            Some('/') => value.push('/'),
            Some(next) => {
                value.push('\\');
                value.push(next);
            }
            None => value.push('\\'),
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{FlagProblem, canonical_flags, check_flags, unescape_slashes};

    #[test]
    fn checks_flags() {
        assert_eq!(check_flags(b"xsmi"), Ok(()));
        assert_eq!(check_flags(b"ii"), Err(FlagProblem::Repeated));
        assert_eq!(check_flags(b"g"), Err(FlagProblem::Unknown));
        assert_eq!(check_flags(b"i2"), Err(FlagProblem::Suffix));
    }

    #[test]
    fn orders_flags_canonically() {
        assert_eq!(canonical_flags(b"xsi"), "isx");
        assert_eq!(canonical_flags(b"gii"), "i");
    }

    #[test]
    fn keeps_escaped_pairs_except_slash() {
        assert_eq!(unescape_slashes(r"a\/b\\c\d"), r"a/b\\c\d");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
