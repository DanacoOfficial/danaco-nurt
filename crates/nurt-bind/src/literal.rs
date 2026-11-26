// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Decoders of literal texts and of catalog default values.
//!
//! A quoted text in a place of type `size`, `duration` or `instant` is
//! converted to the value it spells. A catalog default is a literal of the
//! language in variant A (`"**"`, `120s`, `asc`, `[]`, `all`); it is read into
//! a surface expression and then cast to the type of its parameter like a
//! written argument.

use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use nurt_contracts::ast::{Expr, MAX_SAFE_INTEGER, MapEntry};
use nurt_contracts::spec::{DefaultKind, DefaultValue};

/// Milliseconds of each duration unit.
const DURATION_UNITS: [(&str, u128); 6] = [
    ("ms", 1),
    ("s", 1_000),
    ("m", 60_000),
    ("h", 3_600_000),
    ("d", 86_400_000),
    ("w", 604_800_000),
];

/// Bytes of each size unit: decimal units are powers of 1000, binary units
/// powers of 1024.
const SIZE_UNITS: [(&str, u128); 9] = [
    ("b", 1),
    ("kb", 1_000),
    ("mb", 1_000_000),
    ("gb", 1_000_000_000),
    ("tb", 1_000_000_000_000),
    ("kib", 1 << 10),
    ("mib", 1 << 20),
    ("gib", 1 << 30),
    ("tib", 1 << 40),
];

/// Suffix of a share of the free space, `"10%free"`.
const PERCENT_FREE_SUFFIX: &str = "%free";

/// Base of decimal fractions.
const DECIMAL_BASE: u128 = 10;

/// Decodes a duration such as `30s` or `1h30m` to milliseconds.
pub(crate) fn duration_ms(text: &str) -> Option<u64> {
    let mut rest = text;
    let mut total: u128 = 0;
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let (number, after) = split_number(rest)?;
        let unit_len = after
            .find(|c: char| !c.is_ascii_lowercase())
            .unwrap_or(after.len());
        let (unit, tail) = after.split_at(unit_len);
        let factor = DURATION_UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, factor)| *factor)?;
        total = total.checked_add(scaled(number, factor)?)?;
        rest = tail;
    }
    safe(total)
}

/// Decodes a size such as `10mb` or `1.5kib` to bytes.
pub(crate) fn size_bytes(text: &str) -> Option<u64> {
    let (number, unit) = split_number(text)?;
    let factor = SIZE_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, factor)| *factor)?;
    safe(scaled(number, factor)?)
}

/// Returns `true` for a share of a capacity: `"10%"` or `"10%free"`.
pub(crate) fn is_percent(text: &str) -> bool {
    let number = text
        .strip_suffix(PERCENT_FREE_SUFFIX)
        .or_else(|| text.strip_suffix('%'));
    number.is_some_and(|number| split_number(number).is_some_and(|(_, rest)| rest.is_empty()))
}

/// Converts an instant to RFC 3339 in UTC; a date or a date and time
/// without an offset is read in the local time rules.
pub(crate) fn instant(text: &str, local: &TimeZone) -> Option<String> {
    if let Ok(timestamp) = Timestamp::from_str(text) {
        return Some(timestamp.to_string());
    }
    let datetime = DateTime::from_str(text).ok().or_else(|| {
        Date::from_str(text)
            .ok()
            .map(|date| date.to_datetime(Time::midnight()))
    })?;
    local
        .to_timestamp(datetime)
        .ok()
        .map(|timestamp| timestamp.to_string())
}

/// Number with an optional decimal fraction: the whole part, the fraction
/// digits and the text after the number.
struct Number<'t> {
    whole: &'t str,
    fraction: &'t str,
}

fn split_number(text: &str) -> Option<(Number<'_>, &str)> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
        .unwrap_or(text.len());
    let (digits, rest) = text.split_at(end);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let valid = |part: &str| {
        part.bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    };
    let has_digit = whole.bytes().any(|byte| byte.is_ascii_digit());
    (has_digit && valid(whole) && valid(fraction) && !fraction.contains('_'))
        .then_some((Number { whole, fraction }, rest))
}

/// Multiplies a decimal number by a unit factor; `None` when the result is
/// not a whole number of the base unit.
fn scaled(number: Number<'_>, factor: u128) -> Option<u128> {
    let whole: u128 = number.whole.replace('_', "").parse().ok()?;
    let mut value = whole.checked_mul(factor)?;
    if !number.fraction.is_empty() {
        let digits = u32::try_from(number.fraction.len()).ok()?;
        let denominator = DECIMAL_BASE.checked_pow(digits)?;
        let fraction: u128 = number.fraction.parse().ok()?;
        let part = fraction.checked_mul(factor)?;
        if part % denominator != 0 {
            return None;
        }
        value = value.checked_add(part / denominator)?;
    }
    Some(value)
}

fn safe(value: u128) -> Option<u64> {
    let limit = u128::try_from(MAX_SAFE_INTEGER).ok()?;
    if value > limit {
        return None;
    }
    u64::try_from(value).ok()
}

/// Reads a catalog default value into a surface expression.
pub(crate) fn default_expr(value: &DefaultValue) -> Option<Expr> {
    let mut parser = LiteralParser::new(value.as_str().trim());
    let expr = parser.value()?;
    parser.skip_space();
    parser.at_end().then_some(expr)
}

struct LiteralParser<'t> {
    text: &'t str,
    pos: usize,
}

impl<'t> LiteralParser<'t> {
    fn new(text: &'t str) -> Self {
        Self { text, pos: 0 }
    }

    fn rest(&self) -> &'t str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    fn at_end(&self) -> bool {
        self.rest().is_empty()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn skip_space(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.text.len() - trimmed.len();
    }

    fn eat(&mut self, wanted: char) -> bool {
        self.skip_space();
        if self.peek() == Some(wanted) {
            self.pos += wanted.len_utf8();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Expr> {
        self.skip_space();
        let rest = self.rest();
        match self.peek()? {
            '"' => self.quoted(),
            'r' if rest.starts_with("r\"") || rest.starts_with("r#") => self.raw(),
            '[' => self.list(),
            '{' => self.map(),
            '/' => self.regex(),
            _ => self.bare(),
        }
    }

    fn quoted(&mut self) -> Option<Expr> {
        self.pos += 1;
        let mut text = String::new();
        let mut chars = self.rest().char_indices();
        while let Some((offset, c)) = chars.next() {
            match c {
                '"' => {
                    self.pos += offset + 1;
                    return Some(text_expr(text));
                }
                '\\' => text.push(unescape(chars.next()?.1)?),
                other => text.push(other),
            }
        }
        None
    }

    fn raw(&mut self) -> Option<Expr> {
        let after_r = self.rest().get(1..)?;
        let hashes = after_r.len() - after_r.trim_start_matches('#').len();
        let body = after_r.get(hashes..)?.strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&close)?;
        let text = body.get(..end)?.to_owned();
        self.pos += 1 + hashes + 1 + end + close.len();
        Some(text_expr(text))
    }

    fn list(&mut self) -> Option<Expr> {
        self.pos += 1;
        let mut items = Vec::new();
        if self.eat(']') {
            return Some(Expr::List { items, span: None });
        }
        loop {
            items.push(self.value()?);
            if self.eat(']') {
                return Some(Expr::List { items, span: None });
            }
            if !self.eat(',') {
                return None;
            }
        }
    }

    fn map(&mut self) -> Option<Expr> {
        self.pos += 1;
        let mut entries = Vec::new();
        if self.eat('}') {
            return Some(Expr::Map {
                entries,
                span: None,
            });
        }
        loop {
            self.skip_space();
            let key = self.word();
            if key.is_empty() || !self.eat(':') {
                return None;
            }
            let v = self.value()?;
            entries.push(MapEntry {
                k: key.to_owned(),
                v,
                span: None,
            });
            if self.eat('}') {
                return Some(Expr::Map {
                    entries,
                    span: None,
                });
            }
            if !self.eat(',') {
                return None;
            }
        }
    }

    fn regex(&mut self) -> Option<Expr> {
        self.pos += 1;
        let mut pattern = String::new();
        let mut chars = self.rest().char_indices();
        while let Some((offset, c)) = chars.next() {
            match c {
                '/' => {
                    self.pos += offset + 1;
                    let flags = self.word().to_owned();
                    return Some(Expr::Regex {
                        v: pattern,
                        flags,
                        span: None,
                    });
                }
                '\\' => {
                    let (_, next) = chars.next()?;
                    if next != '/' {
                        pattern.push('\\');
                    }
                    pattern.push(next);
                }
                other => pattern.push(other),
            }
        }
        None
    }

    fn word(&mut self) -> &'t str {
        let rest = self.rest();
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        self.pos += end;
        rest.get(..end).unwrap_or_default()
    }

    fn bare(&mut self) -> Option<Expr> {
        let rest = self.rest();
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, ',' | ']' | '}'))
            .unwrap_or(rest.len());
        let token = rest.get(..end)?;
        self.pos += end;
        bare_expr(token)
    }
}

fn text_expr(v: String) -> Expr {
    Expr::Text { v, span: None }
}

fn unescape(c: char) -> Option<char> {
    match c {
        '"' => Some('"'),
        '\\' => Some('\\'),
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        '0' => Some('\0'),
        _ => None,
    }
}

fn bare_expr(token: &str) -> Option<Expr> {
    match DefaultValue::new(token).kind() {
        DefaultKind::Bool => Some(Expr::Bool {
            v: token == "true",
            span: None,
        }),
        DefaultKind::Null => Some(Expr::Null { span: None }),
        DefaultKind::Int => integer(token).map(|v| Expr::Int { v, span: None }),
        DefaultKind::Number => token
            .replace('_', "")
            .parse()
            .ok()
            .map(|v| Expr::Num { v, span: None }),
        DefaultKind::Size => size_bytes(token).map(|v| Expr::Size { v, span: None }),
        DefaultKind::Duration => duration_ms(token).map(|v| Expr::Dur { v, span: None }),
        DefaultKind::Range => range_expr(token),
        DefaultKind::Word => Some(Expr::Ident {
            v: token.to_owned(),
            span: None,
        }),
        _ => None,
    }
}

fn integer(token: &str) -> Option<i64> {
    let value: i64 = token.replace('_', "").parse().ok()?;
    (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER)
        .contains(&value)
        .then_some(value)
}

fn range_expr(token: &str) -> Option<Expr> {
    let (lo, hi) = token.split_once("..")?;
    let bound = |part: &str| {
        if part.is_empty() {
            Some(None)
        } else {
            integer(part).map(Some)
        }
    };
    Some(Expr::Range {
        lo: bound(lo)?,
        hi: bound(hi)?,
        span: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(literal: &str) -> Option<Expr> {
        default_expr(&DefaultValue::new(literal))
    }

    #[test]
    fn decodes_durations() {
        assert_eq!(duration_ms("30s"), Some(30_000));
        assert_eq!(duration_ms("1h30m"), Some(5_400_000));
        assert_eq!(duration_ms("1.5s"), Some(1_500));
        assert_eq!(duration_ms("250ms"), Some(250));
        assert_eq!(duration_ms("7d"), Some(604_800_000));
        assert_eq!(duration_ms("0s"), Some(0));
        assert_eq!(duration_ms("10"), None);
        assert_eq!(duration_ms("10x"), None);
        assert_eq!(duration_ms(""), None);
        assert_eq!(duration_ms("0.0001s"), None);
    }

    #[test]
    fn decodes_sizes() {
        assert_eq!(size_bytes("10mb"), Some(10_000_000));
        assert_eq!(size_bytes("1.5mb"), Some(1_500_000));
        assert_eq!(size_bytes("2kib"), Some(2_048));
        assert_eq!(size_bytes("1_000b"), Some(1_000));
        assert_eq!(size_bytes("10"), None);
        assert_eq!(size_bytes("10MB"), None);
        assert_eq!(size_bytes("99999999tb"), None);
    }

    #[test]
    fn decodes_day_and_week_durations() {
        assert_eq!(duration_ms("24h"), Some(86_400_000));
        assert_eq!(duration_ms("1w"), Some(604_800_000));
        assert_eq!(duration_ms("0.5w"), Some(302_400_000));
        assert_eq!(duration_ms("1w1d1h"), Some(694_800_000));
        assert_eq!(duration_ms("1W"), None);
    }

    #[test]
    fn decodes_the_largest_exact_values() {
        assert_eq!(
            duration_ms("9007199254740991ms"),
            Some(9_007_199_254_740_991)
        );
        assert_eq!(duration_ms("9007199254740992ms"), None);
        assert_eq!(size_bytes("9007199254740991b"), Some(9_007_199_254_740_991));
        assert_eq!(size_bytes("9007199254740992b"), None);
        assert_eq!(size_bytes("1mb"), Some(1_000_000));
        assert_eq!(size_bytes("0.0005kb"), None);
    }

    #[test]
    fn rejects_units_and_digits_of_multi_byte_text() {
        assert_eq!(duration_ms("1é"), None);
        assert_eq!(duration_ms("1ч"), None);
        assert_eq!(duration_ms("１s"), None);
        assert_eq!(size_bytes("1é"), None);
        assert_eq!(size_bytes("10мб"), None);
        assert_eq!(size_bytes("１０mb"), None);
        assert!(!is_percent("１０%"));
        assert!(!is_percent("10é"));
        assert_eq!(instant("2026-03-01é", &TimeZone::UTC), None);
        assert_eq!(instant("é", &TimeZone::UTC), None);
    }

    #[test]
    fn recognises_percentages() {
        assert!(is_percent("10%"));
        assert!(is_percent("12.5%free"));
        assert!(!is_percent("%"));
        assert!(!is_percent("10"));
    }

    #[test]
    fn converts_instants_to_utc() {
        let utc = TimeZone::UTC;
        assert_eq!(
            instant("2026-03-01T12:00:00Z", &utc).as_deref(),
            Some("2026-03-01T12:00:00Z")
        );
        assert_eq!(
            instant("2026-03-01T14:00:00+02:00", &utc).as_deref(),
            Some("2026-03-01T12:00:00Z")
        );
        assert_eq!(
            instant("2026-03-01T12:00:00", &utc).as_deref(),
            Some("2026-03-01T12:00:00Z")
        );
        assert_eq!(
            instant("2026-03-01", &utc).as_deref(),
            Some("2026-03-01T00:00:00Z")
        );
        assert_eq!(instant("1h", &utc), None);
        let plus_two = TimeZone::fixed(jiff::tz::offset(2));
        assert_eq!(
            instant("2026-03-01T12:00:00", &plus_two).as_deref(),
            Some("2026-03-01T10:00:00Z")
        );
    }

    #[test]
    fn reads_default_literals() {
        assert_eq!(
            parsed("true"),
            Some(Expr::Bool {
                v: true,
                span: None
            })
        );
        assert_eq!(parsed("null"), Some(Expr::Null { span: None }));
        assert_eq!(parsed("500"), Some(Expr::Int { v: 500, span: None }));
        assert_eq!(
            parsed("120s"),
            Some(Expr::Dur {
                v: 120_000,
                span: None
            })
        );
        assert_eq!(parsed("\"**\""), Some(text_expr("**".to_owned())));
        assert_eq!(parsed("\"a\\\"b\""), Some(text_expr("a\"b".to_owned())));
        assert_eq!(parsed("r#\"x\"y\"#"), Some(text_expr("x\"y".to_owned())));
        assert_eq!(
            parsed("asc"),
            Some(Expr::Ident {
                v: "asc".to_owned(),
                span: None
            })
        );
        assert_eq!(
            parsed("1..5"),
            Some(Expr::Range {
                lo: Some(1),
                hi: Some(5),
                span: None
            })
        );
        assert_eq!(
            parsed("[]"),
            Some(Expr::List {
                items: Vec::new(),
                span: None
            })
        );
    }

    #[test]
    fn reads_lists_maps_and_regular_expressions() {
        let Some(Expr::List { items, .. }) = parsed("[path, package, \"x\"]") else {
            panic!("a list is expected");
        };
        assert_eq!(items.len(), 3);
        let Some(Expr::Map { entries, .. }) = parsed("{depth: 1, name: \"a\"}") else {
            panic!("a map is expected");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(
            parsed("/a\\/b\\d/i"),
            Some(Expr::Regex {
                v: "a/b\\d".to_owned(),
                flags: "i".to_owned(),
                span: None
            })
        );
    }

    #[test]
    fn rejects_malformed_literals() {
        assert_eq!(parsed("\"open"), None);
        assert_eq!(parsed("[1, 2"), None);
        assert_eq!(parsed("1 2"), None);
        assert_eq!(parsed("$x"), None);
    }
}
