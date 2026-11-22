// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::config::YamlVersion;

/// Characters that start a YAML indicator and cannot start a plain scalar.
const YAML_INDICATORS: &[char] = &[
    '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
];

/// Type a YAML plain scalar resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlainKind {
    /// Null: `null`, `~` or empty.
    Null,
    /// Boolean.
    Bool,
    /// Integer.
    Int,
    /// Floating point number, infinity or not-a-number.
    Float,
    /// Timestamp (YAML 1.1).
    Timestamp,
    /// Text.
    Str,
}

/// Type the plain scalar `text` resolves to under `version`.
///
/// ```
/// use nurt_contracts::config::YamlVersion;
/// use nurt_doc::value::{PlainKind, yaml_plain_kind};
///
/// assert_eq!(yaml_plain_kind("yes", YamlVersion::V1_1), PlainKind::Bool);
/// assert_eq!(yaml_plain_kind("yes", YamlVersion::V1_2), PlainKind::Str);
/// assert_eq!(yaml_plain_kind("0o17", YamlVersion::V1_2), PlainKind::Int);
/// assert_eq!(yaml_plain_kind("2026-11-22", YamlVersion::V1_1), PlainKind::Timestamp);
/// ```
#[must_use]
pub fn yaml_plain_kind(text: &str, version: YamlVersion) -> PlainKind {
    let yaml11 = version == YamlVersion::V1_1;
    if matches!(text, "" | "~" | "null" | "Null" | "NULL") {
        return PlainKind::Null;
    }
    if matches!(text, "true" | "True" | "TRUE" | "false" | "False" | "FALSE")
        || (yaml11
            && matches!(
                text,
                "y" | "Y" | "yes" | "Yes" | "YES" | "n" | "N" | "no" | "No" | "NO" | "on" | "On"
                    | "ON" | "off" | "Off" | "OFF"
            ))
    {
        return PlainKind::Bool;
    }
    if let Some(kind) = yaml_number_kind(text, yaml11) {
        return kind;
    }
    if yaml11 && is_timestamp(text) {
        return PlainKind::Timestamp;
    }
    PlainKind::Str
}

/// Quoting rules of a format with untyped or loosely typed values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QuoteRules {
    /// YAML plain scalars, for both 1.1 and 1.2 readers.
    Yaml,
    /// INI values (`;` and `#` start comments in common dialects).
    Ini,
    /// Environment files read by shells and `dotenv` readers.
    Env,
}

/// Returns `true` when `text` written without quotes would not be read back
/// as the same text.
///
/// YAML text is quoted when a YAML 1.1 or a YAML 1.2 reader would type it
/// (`yes`, `1.0`, `null`, `2026-11-22`) or when it holds an indicator, a
/// line break, a comment marker or surrounding blanks.
///
/// ```
/// use nurt_doc::value::{QuoteRules, needs_quotes};
///
/// assert!(needs_quotes("on", QuoteRules::Yaml));
/// assert!(needs_quotes("a: b", QuoteRules::Yaml));
/// assert!(!needs_quotes("hello world", QuoteRules::Yaml));
/// assert!(needs_quotes("a b", QuoteRules::Env));
/// assert!(needs_quotes("x ; y", QuoteRules::Ini));
/// ```
#[must_use]
pub fn needs_quotes(text: &str, rules: QuoteRules) -> bool {
    if text.contains(['\n', '\r']) || text.chars().any(|c| c.is_control() && c != '\t') {
        return true;
    }
    let blank_edges = text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace);
    match rules {
        QuoteRules::Yaml => yaml_needs_quotes(text),
        QuoteRules::Ini => {
            blank_edges || text.contains([';', '#', '"']) || text.starts_with(['\'', '['])
        }
        QuoteRules::Env => {
            text.chars().any(|c| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '#' | '\'' | '"' | '$' | '\\' | '`' | ';' | '&' | '|' | '<' | '>' | '(' | ')'
                    )
            })
        }
    }
}

fn yaml_needs_quotes(text: &str) -> bool {
    if yaml_plain_kind(text, YamlVersion::V1_1) != PlainKind::Str
        || yaml_plain_kind(text, YamlVersion::V1_2) != PlainKind::Str
    {
        return true;
    }
    text.starts_with(YAML_INDICATORS)
        || text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.ends_with(':')
        || text.contains(": ")
        || text.contains(" #")
        || text.contains('\t')
        || matches!(text, "<<" | "=")
}

/// Kind of a YAML number lexeme, `None` when the text is not a number.
pub(super) fn yaml_number_kind(text: &str, yaml11: bool) -> Option<PlainKind> {
    if matches!(text, ".nan" | ".NaN" | ".NAN") {
        return Some(PlainKind::Float);
    }
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if matches!(unsigned, ".inf" | ".Inf" | ".INF") {
        return Some(PlainKind::Float);
    }
    if yaml11 { yaml11_number(unsigned) } else { yaml12_number(text, unsigned) }
}

fn digits(text: &str, radix: u32) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_digit(radix))
}

fn digits_or_underscore(text: &str, radix: u32) -> bool {
    text.chars().all(|c| c == '_' || c.is_digit(radix))
}

fn yaml12_number(text: &str, unsigned: &str) -> Option<PlainKind> {
    let signless = text == unsigned;
    if signless
        && (text.strip_prefix("0o").is_some_and(|d| digits(d, 8))
            || text.strip_prefix("0x").is_some_and(|d| digits(d, 16)))
    {
        return Some(PlainKind::Int);
    }
    if digits(unsigned, 10) {
        return Some(PlainKind::Int);
    }
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((whole, fraction)) => {
            (whole.is_empty() && digits(fraction, 10))
                || (digits(whole, 10) && (fraction.is_empty() || digits(fraction, 10)))
        }
        None => digits(mantissa, 10),
    };
    let exponent_ok =
        exponent.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e), 10));
    (mantissa_ok && exponent_ok && (mantissa.contains('.') || exponent.is_some()))
        .then_some(PlainKind::Float)
}

fn yaml11_number(unsigned: &str) -> Option<PlainKind> {
    let starts_digit = unsigned.starts_with(|c: char| c.is_ascii_digit());
    if unsigned.strip_prefix("0b").is_some_and(|d| !d.is_empty() && digits_or_underscore(d, 2))
        || unsigned.strip_prefix("0x").is_some_and(|d| !d.is_empty() && digits_or_underscore(d, 16))
        || (starts_digit && digits_or_underscore(unsigned, 10))
    {
        return Some(PlainKind::Int);
    }
    if starts_digit && !unsigned.starts_with('0') && is_sexagesimal(unsigned) {
        return Some(PlainKind::Int);
    }
    yaml11_float(unsigned)
}

/// Matches `[1-9][0-9_]*(:[0-5]?[0-9])+` without the sign.
fn is_sexagesimal(text: &str) -> bool {
    let mut parts = text.split(':');
    let head_ok = parts.next().is_some_and(|head| digits_or_underscore(head, 10) && !head.is_empty());
    let mut rest = parts.peekable();
    head_ok
        && rest.peek().is_some()
        && rest.all(|part| {
            digits(part, 10) && part.len() <= 2 && part.chars().next().is_some_and(|c| c <= '5' || part.len() == 1)
        })
}

/// Matches `([0-9][0-9_]*)?\.[0-9.]*([eE][-+][0-9]+)?` and the sexagesimal
/// form `[0-9][0-9_]*(:[0-5]?[0-9])+\.[0-9_]*`, without the sign.
fn yaml11_float(unsigned: &str) -> Option<PlainKind> {
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let exponent_ok = exponent.is_none_or(|e| {
        e.strip_prefix(['-', '+']).is_some_and(|d| digits(d, 10))
    });
    let (whole, fraction) = mantissa.split_once('.')?;
    let whole_ok = whole.is_empty()
        || (whole.starts_with(|c: char| c.is_ascii_digit())
            && (digits_or_underscore(whole, 10) || is_sexagesimal(whole)));
    let fraction_ok = fraction.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_');
    let nonempty = !(whole.is_empty() && fraction.is_empty());
    (whole_ok && fraction_ok && exponent_ok && nonempty).then_some(PlainKind::Float)
}

/// YAML 1.1 timestamp: `YYYY-MM-DD` or a date with time.
fn is_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    let year_ok = bytes.len() >= "0000-0-0".len()
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-';
    if !year_ok {
        return false;
    }
    let rest = &text[5..];
    let date_end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    let mut date = rest[..date_end].split('-');
    let (Some(month), Some(day), None) = (date.next(), date.next(), date.next()) else {
        return false;
    };
    let date_ok = (1..=2).contains(&month.len())
        && (1..=2).contains(&day.len())
        && digits(month, 10)
        && digits(day, 10);
    let time = &rest[date_end..];
    date_ok && (time.is_empty() || is_time_part(time))
}

/// Matches `([Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(\.[0-9]*)?` followed by an
/// optional zone.
fn is_time_part(text: &str) -> bool {
    let time = text
        .strip_prefix(['T', 't'])
        .unwrap_or_else(|| text.trim_start_matches([' ', '\t']));
    if time.len() == text.len() {
        return false;
    }
    let clock_end = time
        .find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .unwrap_or(time.len());
    let (clock, zone) = time.split_at(clock_end);
    let (hms, _) = clock.split_once('.').unwrap_or((clock, ""));
    let parts: Vec<&str> = hms.split(':').collect();
    let clock_ok = parts.len() == 3
        && (1..=2).contains(&parts[0].len())
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|part| digits(part, 10));
    let zone = zone.trim_start_matches([' ', '\t']);
    let zone_ok = zone.is_empty()
        || zone == "Z"
        || zone.strip_prefix(['+', '-']).is_some_and(|offset| {
            let (hours, minutes) = offset.split_once(':').unwrap_or((offset, "00"));
            (1..=2).contains(&hours.len()) && digits(hours, 10) && minutes.len() == 2 && digits(minutes, 10)
        });
    clock_ok && zone_ok
}
