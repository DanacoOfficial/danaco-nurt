// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::print::literal
// Responsibility:   Canonical spelling of literals, identifiers and names.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Canonical spelling of literals, identifiers and names.

use std::fmt::Write as _;

use nurt_contracts::keypath::{KeyPath, KeySegment};

/// Largest integer that canonical JSON represents exactly, `2^53 - 1`.
pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Longest identifier accepted by the lexer, in bytes.
pub(crate) const MAX_IDENTIFIER_LEN: usize = 64;

/// Longest command name: two identifiers joined by `.`.
pub(crate) const MAX_COMMAND_NAME_LEN: usize = 2 * MAX_IDENTIFIER_LEN + 1;

/// Reserved words; never valid as identifiers, command segments or `let` names.
pub(crate) const KEYWORDS: [&str; 8] = ["let", "true", "false", "null", "and", "or", "not", "in"];

/// Largest number of fraction digits in a size such as `1.125kb`.
const MAX_SIZE_FRACTION_DIGITS: u32 = 3;

/// Size units in tie-break order: decimal before binary, larger before smaller.
const SIZE_UNITS: [(&str, u64); 9] = [
    ("tb", 1_000_000_000_000),
    ("gb", 1_000_000_000),
    ("mb", 1_000_000),
    ("kb", 1_000),
    ("b", 1),
    ("tib", 1 << 40),
    ("gib", 1 << 30),
    ("mib", 1 << 20),
    ("kib", 1 << 10),
];

/// Duration units from the largest, in milliseconds.
const DURATION_UNITS: [(&str, u64); 5] = [
    ("d", 86_400_000),
    ("h", 3_600_000),
    ("m", 60_000),
    ("s", 1_000),
    ("ms", 1),
];

/// Regular expression flags in canonical order.
const REGEX_FLAGS: &str = "imsx";

/// Returns `true` for `[a-z][a-z0-9_]*` of at most [`MAX_IDENTIFIER_LEN`]
/// bytes; reserved words match.
pub(crate) fn is_identifier_shape(text: &str) -> bool {
    let mut chars = text.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    first
        && text.len() <= MAX_IDENTIFIER_LEN
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Returns `true` for a reserved word.
pub(crate) fn is_keyword(text: &str) -> bool {
    KEYWORDS.contains(&text)
}

/// Returns `true` for an identifier that is not a reserved word.
pub(crate) fn is_identifier(text: &str) -> bool {
    is_identifier_shape(text) && !is_keyword(text)
}

/// Returns `true` for a command name: identifiers joined by `.`.
pub(crate) fn is_command_name(text: &str) -> bool {
    text.len() <= MAX_COMMAND_NAME_LEN && text.split('.').all(is_identifier)
}

/// Returns `true` when an integer lies within `0..=2^53 - 1`.
pub(crate) fn is_safe_integer(value: i64) -> bool {
    u64::try_from(value).is_ok_and(|v| v <= MAX_SAFE_INTEGER)
}

/// Spells a text literal.
///
/// The raw form `r#"…"#` is chosen when the text contains `"` or `\` and no
/// control character (Unicode category Cc) other than tab and line feed; the
/// number of `#` is the smallest one for which `"` followed by that many `#`
/// does not occur in the text. Otherwise the quoted form escapes `\ " LF CR
/// TAB NUL` with their short escapes and every other control character as
/// `\u{hex}`.
pub(crate) fn text(value: &str) -> String {
    let needs_escape = value.contains(['"', '\\']);
    let raw_safe = !value
        .chars()
        .any(|c| c.is_control() && c != '\t' && c != '\n');
    if needs_escape && raw_safe {
        raw_text(value)
    } else {
        quoted_text(value)
    }
}

fn raw_text(value: &str) -> String {
    let mut hashes = String::new();
    while value.contains(&format!("\"{hashes}")) {
        hashes.push('#');
    }
    format!("r{hashes}\"{value}\"{hashes}")
}

fn quoted_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Spells a map key: the identifier when it has the identifier shape,
/// reserved words included, otherwise a text literal.
pub(crate) fn map_key(key: &str) -> String {
    if is_identifier_shape(key) {
        key.to_owned()
    } else {
        text(key)
    }
}

/// Problem that makes a regular expression pattern unprintable, if any.
pub(crate) fn regex_problem(pattern: &str, flags: &str) -> Option<&'static str> {
    if pattern.is_empty() {
        return Some("has an empty pattern; `//` is not a regular expression");
    }
    if pattern.contains(['\n', '\r']) {
        return Some("contains a line break; a regular expression is written on one line");
    }
    if !canonical_flags(flags).eq(flags.chars()) {
        return Some("has flags outside `imsx` or out of order; write them in the order `imsx`");
    }
    let mut backslashes = 0_usize;
    for c in pattern.chars() {
        if c == '/' && backslashes % 2 == 1 {
            return Some("has an odd number of `\\` before `/`; double the backslash");
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
    }
    if backslashes % 2 == 1 {
        return Some("ends with an odd number of `\\`; double the last backslash");
    }
    None
}

fn canonical_flags(flags: &str) -> impl Iterator<Item = char> + '_ {
    REGEX_FLAGS.chars().filter(|flag| flags.contains(*flag))
}

/// Spells a regular expression: `/pattern/flags` with every `/` of the
/// pattern written `\/` and the known flags in the order `imsx`.
pub(crate) fn regex(pattern: &str, flags: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + flags.len() + 2);
    out.push('/');
    for c in pattern.chars() {
        if c == '/' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('/');
    out.extend(canonical_flags(flags));
    out
}

/// Spells a number with a fraction: the shortest decimal that reads back to
/// the same value, always with a fractional part and never with an exponent.
///
/// Non-finite values have no spelling and print as `null`.
pub(crate) fn number(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_owned();
    }
    if value == 0.0 {
        return "0.0".to_owned();
    }
    let mut out = value.to_string();
    if !out.contains('.') {
        out.push_str(".0");
    }
    out
}

/// Spells a size in bytes: the shortest of `n<unit>` and `n.f<unit>` whose
/// value is exactly `bytes`; ties prefer decimal units, then larger units.
pub(crate) fn size(bytes: u64) -> String {
    let mut best: Option<String> = None;
    for (unit, factor) in SIZE_UNITS {
        let Some(candidate) = size_in_unit(bytes, unit, factor) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| candidate.len() < b.len()) {
            best = Some(candidate);
        }
    }
    best.unwrap_or_else(|| format!("{bytes}b"))
}

fn size_in_unit(bytes: u64, unit: &str, factor: u64) -> Option<String> {
    let factor = u128::from(factor);
    (0..=MAX_SIZE_FRACTION_DIGITS).find_map(|digits| {
        let scale = 10_u128.pow(digits);
        let scaled = u128::from(bytes) * scale;
        if scaled % factor != 0 {
            return None;
        }
        let units = scaled / factor;
        Some(if digits == 0 {
            format!("{units}{unit}")
        } else {
            let width = usize::try_from(digits).unwrap_or_default();
            format!(
                "{}.{:0width$}{unit}",
                units / scale,
                units % scale,
                width = width
            )
        })
    })
}

/// Spells a duration in milliseconds: units `d h m s ms`, each at most once,
/// greedy from the largest; `0s` for zero.
pub(crate) fn duration(millis: u64) -> String {
    if millis == 0 {
        return "0s".to_owned();
    }
    let mut out = String::new();
    let mut rest = millis;
    for (unit, factor) in DURATION_UNITS {
        let count = rest / factor;
        if count > 0 {
            let _ = write!(out, "{count}{unit}");
            rest %= factor;
        }
    }
    out
}

/// Spells one member name of a key path, quoted as `["…"]` when the bare
/// form would be read differently.
pub(crate) fn key_name(name: &str) -> String {
    KeyPath::new(None, vec![KeySegment::Key(name.to_owned())]).map_or_else(
        |_| format!("[{}]", quoted_text(name)),
        |key| key.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check<T: Copy + std::fmt::Debug>(cases: &[(T, &str)], spell: impl Fn(T) -> String) {
        for (input, expected) in cases {
            assert_eq!(spell(*input), *expected, "{input:?}");
        }
    }

    #[test]
    fn text_uses_short_escapes_and_minimal_unicode_escapes() {
        check(
            &[
                ("a\nb\tc", r#""a\nb\tc""#),
                ("\r\0", r#""\r\0""#),
                ("\u{7}\u{1b}\u{85}", r#""\u{7}\u{1b}\u{85}""#),
                ("Zürich", "\"Zürich\""),
                ("a\"\r", r#""a\"\r""#),
                ("\\\u{0}", r#""\\\0""#),
            ],
            text,
        );
    }

    #[test]
    fn text_prefers_raw_form_with_the_fewest_hashes() {
        check(
            &[
                (r"C:\proj", r#"r"C:\proj""#),
                (r#"import "fmt""#, r##"r#"import "fmt""#"##),
                ("a\"#b", "r##\"a\"#b\"##"),
                ("one \"1\"\ntwo\t", "r#\"one \"1\"\ntwo\t\"#"),
            ],
            text,
        );
    }

    #[test]
    fn map_key_is_identifier_or_text() {
        let long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
        let long_text = text(&long);
        check(
            &[
                ("replace", "replace"),
                ("in", "in"),
                ("key with space", "\"key with space\""),
                ("Upper", "\"Upper\""),
                (long.as_str(), long_text.as_str()),
            ],
            map_key,
        );
    }

    #[test]
    fn regex_escapes_slashes_and_orders_flags() {
        assert_eq!(regex("oldName\\(", ""), "/oldName\\(/");
        assert_eq!(regex("a/b", "xi"), "/a\\/b/ix");
        let printable = [("a/b", "ix"), ("a\\\\/b\\\\", "")];
        for (pattern, flags) in printable {
            assert_eq!(regex_problem(pattern, flags), None, "{pattern:?}");
        }
        let unprintable = [
            ("a", "xi"),
            ("a", "u"),
            ("", ""),
            ("a\nb", ""),
            ("a\\/b", ""),
            ("a\\", ""),
        ];
        for (pattern, flags) in unprintable {
            assert!(
                regex_problem(pattern, flags).is_some(),
                "{pattern:?} {flags:?}"
            );
        }
    }

    #[test]
    fn number_is_shortest_without_exponent() {
        check(
            &[
                (3.0, "3.0"),
                (0.25, "0.25"),
                (0.1, "0.1"),
                (1e21, "1000000000000000000000.0"),
                (-0.0, "0.0"),
                (f64::NAN, "null"),
            ],
            number,
        );
        let tiny = number(5e-324);
        assert!(tiny.starts_with("0.000") && tiny.ends_with('5'));
        assert_eq!(tiny.parse::<f64>().ok(), Some(5e-324));
    }

    #[test]
    fn size_picks_shortest_spelling_with_decimal_and_larger_unit_ties() {
        check(
            &[
                (0, "0b"),
                (1, "1b"),
                (512, "512b"),
                (1000, "1kb"),
                (1024, "1kib"),
                (1500, "1.5kb"),
                (1536, "1536b"),
                (2_000, "2kb"),
                (1_088, "1088b"),
                (1_125, "1125b"),
                (1_000_000, "1mb"),
                (1_572_864, "1.5mib"),
                (10 * 1024 * 1024, "10mib"),
                (1_234_500, "1234.5kb"),
                (1_000_100, "1000.1kb"),
                (1_000_001, "1000001b"),
                (1_000_000_000_000, "1tb"),
            ],
            size,
        );
        assert_eq!(size(u64::MAX), format!("{}b", u64::MAX));
    }

    #[test]
    fn duration_is_greedy_without_weeks() {
        check(
            &[
                (0, "0s"),
                (250, "250ms"),
                (90_000, "1m30s"),
                (86_400_000, "1d"),
                (7 * 86_400_000, "7d"),
                (5_400_000, "1h30m"),
                (90_061_001, "1d1h1m1s1ms"),
            ],
            duration,
        );
    }

    #[test]
    fn identifiers_and_command_names() {
        let identifiers = [
            ("exit_code", true),
            ("in", false),
            ("Name", false),
            ("1a", false),
            ("", false),
        ];
        for (name, valid) in identifiers {
            assert_eq!(is_identifier(name), valid, "{name:?}");
        }
        assert!(is_identifier_shape("in"));
        let commands = [
            ("git.status", true),
            ("git..status", false),
            ("not.status", false),
        ];
        for (name, valid) in commands {
            assert_eq!(is_command_name(name), valid, "{name:?}");
        }
        assert!(is_safe_integer(9_007_199_254_740_991));
        assert!(!is_safe_integer(9_007_199_254_740_992));
        assert!(!is_safe_integer(-1));
    }

    #[test]
    fn key_name_quotes_names_with_separators() {
        assert_eq!(key_name("scripts"), "scripts");
        assert!(key_name("a.b").starts_with('['));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
