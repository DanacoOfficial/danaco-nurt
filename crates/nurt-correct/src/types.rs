// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction of literals in typed places (`T01`–`T09`).
//!
//! Each rule rewrites the notation of a literal that does not have the form
//! of its place: a size with a separated or spelled-out unit, a duration in
//! words or ISO 8601, a boolean word, an integer with thousands separators, a
//! range with another separator, a text where a regular expression is
//! expected, a relative or local instant, or another name of an enumeration
//! value. A rule may change the notation of a unit; it never changes the
//! value of a number.

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use nurt_contracts::bound::BoundExpr;

use crate::distance::Options;
use crate::policy::Class;
use crate::record::{Correction, CorrectionKind, RuleId};
use crate::tables::{Tables, UnitKind};
use crate::values::{self, Match};

/// Commands whose pattern given as text is matched literally, not as a
/// regular expression.
pub const LITERAL_PATTERN_COMMANDS: [&str; 2] = ["search", "replace"];

const MILLIS_PER_SECOND: u64 = 1_000;
const MILLIS_PER_MINUTE: u64 = 60 * MILLIS_PER_SECOND;
const MILLIS_PER_HOUR: u64 = 60 * MILLIS_PER_MINUTE;
const MILLIS_PER_DAY: u64 = 24 * MILLIS_PER_HOUR;
const MILLIS_PER_WEEK: u64 = 7 * MILLIS_PER_DAY;
const DECIMAL_BASE: u128 = 1_000;
const BINARY_BASE: u128 = 1_024;
const THOUSANDS_GROUP: usize = 3;
const INSTANT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%:z";

/// Duration units of canonical text, largest first, with their length in
/// milliseconds.
const DURATION_UNITS: [(&str, u64); 6] = [
    ("w", MILLIS_PER_WEEK),
    ("d", MILLIS_PER_DAY),
    ("h", MILLIS_PER_HOUR),
    ("m", MILLIS_PER_MINUTE),
    ("s", MILLIS_PER_SECOND),
    ("ms", 1),
];

/// Units used when printing a duration: weeks are written as days.
const PRINTED_DURATION_UNITS: [(&str, u64); 5] = [
    ("d", MILLIS_PER_DAY),
    ("h", MILLIS_PER_HOUR),
    ("m", MILLIS_PER_MINUTE),
    ("s", MILLIS_PER_SECOND),
    ("ms", 1),
];

/// Type expected in the place of a literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeTarget {
    /// `bool`.
    Bool,
    /// `int`.
    Int,
    /// `number`.
    Number,
    /// `size`.
    Size,
    /// `duration`.
    Duration,
    /// Integer range.
    Range,
    /// `regex`.
    Regex,
    /// `instant`.
    Instant,
    /// Enumeration with the given values.
    Enum(Vec<String>),
    /// `text`.
    Text,
}

/// A literal as written in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written<'a> {
    /// Text of the literal without quotation marks.
    pub text: &'a str,
    /// The literal was written in quotation marks.
    pub quoted: bool,
}

impl<'a> Written<'a> {
    /// A literal written without quotation marks.
    #[must_use]
    pub const fn bare(text: &'a str) -> Self {
        Self {
            text,
            quoted: false,
        }
    }

    /// A literal written in quotation marks.
    #[must_use]
    pub const fn quoted(text: &'a str) -> Self {
        Self { text, quoted: true }
    }

    fn display(&self) -> String {
        if self.quoted {
            format!("\"{}\"", self.text)
        } else {
            self.text.to_owned()
        }
    }
}

/// Conditions of a correction that do not come from the literal.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// Correction tables.
    pub tables: &'a Tables,
    /// Time zone of local instants.
    pub time_zone: &'a TimeZone,
    /// Name of the stage that receives the literal.
    pub command: Option<&'a str>,
    /// Options of name matching for enumeration values.
    pub options: Options,
}

/// Another reading of a probable correction.
#[derive(Debug, Clone, PartialEq)]
pub struct Alternative {
    /// Bound value of the reading.
    pub value: BoundExpr,
    /// Canonical text of the reading.
    pub text: String,
}

/// A literal converted to its place.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// Bound value.
    pub value: BoundExpr,
    /// Rule that applies.
    pub rule: RuleId,
    /// Certainty class.
    pub class: Class,
    /// Literal as written.
    pub from: String,
    /// Canonical text of the value.
    pub to: String,
    /// Kind of the corrected element.
    pub kind: CorrectionKind,
    /// Other readings, for a probable correction.
    pub alternatives: Vec<Alternative>,
}

impl Converted {
    /// The correction record of the conversion.
    #[must_use]
    pub fn correction(&self) -> Correction {
        Correction::new(
            self.rule,
            self.class,
            self.from.clone(),
            self.to.clone(),
            self.kind,
        )
        .with_alternatives(
            self.alternatives
                .iter()
                .map(|alternative| alternative.text.clone())
                .collect(),
        )
    }
}

/// A literal that cannot be corrected, with the explanation for
/// `E_UNCORRECTABLE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncorrectable {
    /// Rule that recognized the literal.
    pub rule: RuleId,
    /// Literal as written.
    pub found: String,
    /// What is wrong and how to write it.
    pub problem: String,
}

/// Result of a type rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The literal is converted.
    Converted(Converted),
    /// The literal is recognized but has no value in the place.
    Uncorrectable(Uncorrectable),
    /// No rule applies; the literal is either valid as written or a type
    /// error.
    NotApplicable,
}

/// Applies the type rule of `target` to the literal `written`.
///
/// # Examples
///
/// ```
/// use jiff::tz::TimeZone;
/// use nurt_contracts::bound::BoundExpr;
/// use nurt_correct::distance::Options;
/// use nurt_correct::tables::load_embedded;
/// use nurt_correct::types::{Context, Outcome, TypeTarget, Written, correct};
///
/// let tables = load_embedded().unwrap();
/// let context = Context { tables: &tables, time_zone: &TimeZone::UTC, command: None, options: Options::default() };
/// let Outcome::Converted(size) = correct(Written::quoted("10 MB"), &TypeTarget::Size, &context) else {
///     panic!("expected a conversion");
/// };
/// assert_eq!(size.value, BoundExpr::Size { v: 10_000_000 });
/// assert_eq!(size.correction().to_string(), "T01  \"10 MB\" → 10mb  size");
/// ```
#[must_use]
pub fn correct(written: Written<'_>, target: &TypeTarget, context: &Context<'_>) -> Outcome {
    match target {
        TypeTarget::Size => size(written, context.tables),
        TypeTarget::Duration => duration(written, context.tables),
        TypeTarget::Bool => boolean(written, context.tables),
        TypeTarget::Int => integer(written),
        TypeTarget::Range => range(written),
        TypeTarget::Regex => regex(written, context.command),
        TypeTarget::Instant => instant(written, context.tables, context.time_zone),
        TypeTarget::Enum(values) => enumeration(written, values, context),
        TypeTarget::Number | TypeTarget::Text => Outcome::NotApplicable,
    }
}

/// `T01`, `T02`: a size with a separated, upper-case, spelled-out or
/// single-letter unit.
#[must_use]
pub fn size(written: Written<'_>, tables: &Tables) -> Outcome {
    let Some((number, unit)) = split_number(written.text) else {
        return Outcome::NotApplicable;
    };
    if let Some(entry) = tables.unit(UnitKind::Size, unit) {
        let Some(to) = entry.to.as_deref() else {
            return Outcome::NotApplicable;
        };
        let text = format!("{}{to}", number.text);
        if text == written.text {
            return Outcome::NotApplicable;
        }
        return size_value(&number, to).map_or(Outcome::NotApplicable, |bytes| {
            Outcome::Converted(Converted {
                value: BoundExpr::Size { v: bytes },
                rule: entry.rule,
                class: entry.class.unwrap_or(Class::Certain),
                from: written.display(),
                to: text,
                kind: CorrectionKind::Size,
                alternatives: Vec::new(),
            })
        });
    }
    tables
        .unit(UnitKind::SizeLetter, unit)
        .and_then(|entry| {
            let to = entry.to.as_deref()?;
            let bytes = size_value(&number, to)?;
            let alternatives = entry
                .alternative
                .as_deref()
                .and_then(|other| {
                    size_value(&number, other).map(|v| Alternative {
                        value: BoundExpr::Size { v },
                        text: format!("{}{other}", number.text),
                    })
                })
                .into_iter()
                .collect();
            Some(Outcome::Converted(Converted {
                value: BoundExpr::Size { v: bytes },
                rule: entry.rule,
                class: entry.class.unwrap_or(Class::Probable),
                from: written.display(),
                to: format!("{}{to}", number.text),
                kind: CorrectionKind::Size,
                alternatives,
            }))
        })
        .unwrap_or(Outcome::NotApplicable)
}

/// `T03`: a duration in words, abbreviations or ISO 8601.
#[must_use]
pub fn duration(written: Written<'_>, tables: &Tables) -> Outcome {
    let text = written.text.trim();
    if parse_canonical_duration(text).is_some() {
        return Outcome::NotApplicable;
    }
    let parsed = if text.starts_with(['P', 'p']) {
        parse_iso_duration(text)
    } else {
        parse_word_duration(text, tables)
    };
    match parsed {
        Some(Ok(millis)) => Outcome::Converted(Converted {
            value: BoundExpr::Dur { v: millis },
            rule: RuleId::T03,
            class: Class::Certain,
            from: written.display(),
            to: format_duration(millis),
            kind: CorrectionKind::Duration,
            alternatives: Vec::new(),
        }),
        Some(Err(problem)) => Outcome::Uncorrectable(Uncorrectable {
            rule: RuleId::T03,
            found: written.display(),
            problem,
        }),
        None => Outcome::NotApplicable,
    }
}

/// `T04`: a boolean word or a boolean written as text.
#[must_use]
pub fn boolean(written: Written<'_>, tables: &Tables) -> Outcome {
    let Some(entry) = tables.unit(UnitKind::Bool, written.text.trim()) else {
        return Outcome::NotApplicable;
    };
    let Some(to) = entry.to.as_deref() else {
        return Outcome::NotApplicable;
    };
    if !written.quoted && written.text == to {
        return Outcome::NotApplicable;
    }
    Outcome::Converted(Converted {
        value: BoundExpr::Bool { v: to == "true" },
        rule: entry.rule,
        class: entry.class.unwrap_or(Class::Certain),
        from: written.display(),
        to: to.to_owned(),
        kind: CorrectionKind::Bool,
        alternatives: Vec::new(),
    })
}

/// `T05`: an integer written with a zero fraction or thousands separators.
#[must_use]
pub fn integer(written: Written<'_>) -> Outcome {
    let text = written.text.trim();
    if text.parse::<i64>().is_ok() {
        return if written.quoted {
            plain_integer(written, text)
        } else {
            Outcome::NotApplicable
        };
    }
    let (negative, digits) = text
        .strip_prefix('-')
        .map_or((false, text), |rest| (true, rest));
    let Some(plain) = zero_fraction(digits).or_else(|| without_thousands(digits)) else {
        return Outcome::NotApplicable;
    };
    let signed = if negative { format!("-{plain}") } else { plain };
    plain_integer(written, &signed)
}

fn plain_integer(written: Written<'_>, digits: &str) -> Outcome {
    digits.parse::<i64>().map_or(Outcome::NotApplicable, |v| {
        Outcome::Converted(Converted {
            value: BoundExpr::Int { v },
            rule: RuleId::T05,
            class: Class::Certain,
            from: written.display(),
            to: v.to_string(),
            kind: CorrectionKind::Int,
            alternatives: Vec::new(),
        })
    })
}

/// `T06`: a range written with `-`, `:` or `,`, or as `>=n` and `<=n`.
#[must_use]
pub fn range(written: Written<'_>) -> Outcome {
    let text = written.text.trim();
    let bounds = if let Some(low) = text.strip_prefix(">=") {
        parse_bound(low).map(|lo| (Some(lo), None))
    } else if let Some(high) = text.strip_prefix("<=") {
        parse_bound(high).map(|hi| (None, Some(hi)))
    } else {
        split_range(text)
    };
    let Some((lo, hi)) = bounds else {
        return Outcome::NotApplicable;
    };
    let to = format!(
        "{}..{}",
        lo.map(|v| v.to_string()).unwrap_or_default(),
        hi.map(|v| v.to_string()).unwrap_or_default()
    );
    Outcome::Converted(Converted {
        value: BoundExpr::Range { lo, hi },
        rule: RuleId::T06,
        class: Class::Certain,
        from: written.display(),
        to,
        kind: CorrectionKind::Range,
        alternatives: Vec::new(),
    })
}

/// `T07`: a text where a regular expression is expected, outside commands
/// that match text patterns literally.
#[must_use]
pub fn regex(written: Written<'_>, command: Option<&str>) -> Outcome {
    if !written.quoted || command.is_some_and(|name| LITERAL_PATTERN_COMMANDS.contains(&name)) {
        return Outcome::NotApplicable;
    }
    if regex::Regex::new(written.text).is_err() {
        return Outcome::NotApplicable;
    }
    Outcome::Converted(Converted {
        value: BoundExpr::Regex {
            v: written.text.to_owned(),
            flags: String::new(),
        },
        rule: RuleId::T07,
        class: Class::Probable,
        from: written.display(),
        to: format!("/{}/", written.text.replace('/', "\\/")),
        kind: CorrectionKind::Regex,
        alternatives: Vec::new(),
    })
}

/// `T08`: a relative instant in words or a local date and time.
#[must_use]
pub fn instant(written: Written<'_>, tables: &Tables, time_zone: &TimeZone) -> Outcome {
    let words = written
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(entry) = tables.unit(UnitKind::Instant, &words) {
        let Some((to, millis)) = entry
            .to
            .as_deref()
            .and_then(|to| parse_ago(to).map(|millis| (to, millis)))
        else {
            return Outcome::NotApplicable;
        };
        return Outcome::Converted(Converted {
            value: BoundExpr::Call {
                function: "ago".to_owned(),
                args: vec![BoundExpr::Dur { v: millis }],
            },
            rule: entry.rule,
            class: entry.class.unwrap_or(Class::Probable),
            from: written.display(),
            to: to.to_owned(),
            kind: CorrectionKind::Instant,
            alternatives: Vec::new(),
        });
    }
    local_instant(written.text.trim(), time_zone).map_or(Outcome::NotApplicable, |text| {
        Outcome::Converted(Converted {
            value: BoundExpr::Instant { v: text.clone() },
            rule: RuleId::T08,
            class: Class::Probable,
            from: written.display(),
            to: format!("\"{text}\""),
            kind: CorrectionKind::Instant,
            alternatives: Vec::new(),
        })
    })
}

/// `T09` and `V01`: another name of an enumeration value.
#[must_use]
pub fn enumeration(written: Written<'_>, allowed: &[String], context: &Context<'_>) -> Outcome {
    let names: Vec<&str> = allowed.iter().map(String::as_str).collect();
    match values::value(written.text, &names, context.tables, &context.options) {
        Match::Corrected(found) => Outcome::Converted(Converted {
            value: BoundExpr::Enum {
                v: found.to.clone(),
            },
            rule: found.rule,
            class: found.class,
            from: written.display(),
            to: found.to,
            kind: CorrectionKind::Value,
            alternatives: Vec::new(),
        }),
        Match::Same | Match::Tie(_) | Match::Unknown => Outcome::NotApplicable,
    }
}

/// Decimal number as written: digits with an optional fraction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decimal<'a> {
    text: &'a str,
    mantissa: u128,
    scale: u32,
}

fn parse_decimal(text: &str) -> Option<Decimal<'_>> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    if text.ends_with('.') {
        return None;
    }
    let mantissa = format!("{whole}{fraction}").parse::<u128>().ok()?;
    let scale = u32::try_from(fraction.len()).ok()?;
    Some(Decimal {
        text,
        mantissa,
        scale,
    })
}

impl Decimal<'_> {
    /// `self × unit`, when the product is a whole number.
    fn times(&self, unit: u128) -> Option<u64> {
        let product = self.mantissa.checked_mul(unit)?;
        let divisor = 10u128.checked_pow(self.scale)?;
        if product % divisor != 0 {
            return None;
        }
        u64::try_from(product / divisor).ok()
    }
}

fn split_number(text: &str) -> Option<(Decimal<'_>, &str)> {
    let text = text.trim();
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, rest) = text.split_at(end);
    let unit = rest.trim_start_matches(|c: char| c.is_whitespace());
    if unit.is_empty() || unit.contains(char::is_whitespace) {
        return None;
    }
    Some((parse_decimal(number)?, unit))
}

fn size_value(number: &Decimal<'_>, unit: &str) -> Option<u64> {
    let factor = match unit {
        "b" => 1,
        "kb" => DECIMAL_BASE,
        "mb" => DECIMAL_BASE.pow(2),
        "gb" => DECIMAL_BASE.pow(3),
        "tb" => DECIMAL_BASE.pow(4),
        "kib" => BINARY_BASE,
        "mib" => BINARY_BASE.pow(2),
        "gib" => BINARY_BASE.pow(3),
        "tib" => BINARY_BASE.pow(4),
        _ => return None,
    };
    number.times(factor)
}

fn unit_millis(unit: &str) -> Option<u64> {
    DURATION_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|(_, millis)| *millis)
}

/// Parses canonical duration text such as `1h30m`; `None` when the text is
/// not canonical.
pub(crate) fn parse_canonical_duration(text: &str) -> Option<u64> {
    let mut rest = text;
    let mut total: u64 = 0;
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let (number, tail) = rest.split_at(digits);
        let letters = tail
            .find(|c: char| !c.is_ascii_lowercase())
            .unwrap_or(tail.len());
        let (unit, next) = tail.split_at(letters);
        let part = number
            .parse::<u64>()
            .ok()?
            .checked_mul(unit_millis(unit)?)?;
        total = total.checked_add(part)?;
        rest = next;
    }
    Some(total)
}

fn parse_word_duration(text: &str, tables: &Tables) -> Option<Result<u64, String>> {
    let normalized = text.replace(',', " ");
    let tokens: Vec<&str> = normalized
        .split_whitespace()
        .filter(|token| *token != "and")
        .collect();
    let mut parts = Vec::new();
    for token in tokens {
        let end = token
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(token.len());
        let (number, unit) = token.split_at(end);
        match (number.is_empty(), unit.is_empty()) {
            (false, false) => parts.push((number, unit)),
            (false, true) => parts.push((number, "")),
            (true, false) => match parts.last_mut() {
                Some(last) if last.1.is_empty() => last.1 = unit,
                _ => return None,
            },
            (true, true) => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    for (number, unit) in parts {
        let entry = tables.unit(UnitKind::Duration, unit)?;
        if entry.error.is_some() {
            return Some(Err(entry.problem.clone().unwrap_or_default()));
        }
        let millis = unit_millis(entry.to.as_deref()?)?;
        total = total.checked_add(parse_decimal(number)?.times(u128::from(millis))?)?;
    }
    Some(Ok(total))
}

fn parse_iso_duration(text: &str) -> Option<Result<u64, String>> {
    let upper = text.to_ascii_uppercase();
    let body = upper.strip_prefix('P')?;
    let (date, time) = body.split_once('T').unwrap_or((body, ""));
    if date.contains(['Y', 'M']) {
        return Some(Err(
            "months and years have no fixed length; write the duration in days or weeks".to_owned(),
        ));
    }
    if body.is_empty() || (body.contains('T') && time.is_empty()) {
        return None;
    }
    let date_millis = iso_components(date, &[('W', MILLIS_PER_WEEK), ('D', MILLIS_PER_DAY)])?;
    let time_millis = iso_components(
        time,
        &[
            ('H', MILLIS_PER_HOUR),
            ('M', MILLIS_PER_MINUTE),
            ('S', MILLIS_PER_SECOND),
        ],
    )?;
    Some(Ok(date_millis.checked_add(time_millis)?))
}

fn iso_components(text: &str, units: &[(char, u64)]) -> Option<u64> {
    let mut rest = text;
    let mut total: u64 = 0;
    let mut allowed = units;
    while !rest.is_empty() {
        let end = rest.find(|c: char| c.is_ascii_alphabetic())?;
        let (number, tail) = rest.split_at(end);
        let designator = tail.chars().next()?;
        let position = allowed.iter().position(|(unit, _)| *unit == designator)?;
        let millis = allowed.get(position)?.1;
        total = total
            .checked_add(parse_decimal(&number.replace(',', "."))?.times(u128::from(millis))?)?;
        allowed = allowed.get(position + 1..)?;
        rest = tail.get(designator.len_utf8()..)?;
    }
    Some(total)
}

/// Canonical text of a duration in milliseconds, largest units first.
#[must_use]
pub fn format_duration(millis: u64) -> String {
    if millis == 0 {
        return "0s".to_owned();
    }
    let mut rest = millis;
    let mut text = String::new();
    for (unit, size) in PRINTED_DURATION_UNITS {
        let count = rest / size;
        if count > 0 {
            text.push_str(&format!("{count}{unit}"));
            rest %= size;
        }
    }
    text
}

fn parse_ago(text: &str) -> Option<u64> {
    parse_canonical_duration(text.strip_prefix("ago(")?.strip_suffix(')')?)
}

fn zero_fraction(digits: &str) -> Option<String> {
    let (whole, fraction) = digits.split_once('.')?;
    let valid = !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && !fraction.is_empty()
        && fraction.len() != THOUSANDS_GROUP
        && fraction.bytes().all(|b| b == b'0');
    valid.then(|| whole.to_owned())
}

fn without_thousands(digits: &str) -> Option<String> {
    let separator = digits.chars().find(|c| !c.is_ascii_digit())?;
    if !matches!(separator, ',' | ' ' | '\u{a0}' | '\u{202f}' | '_') {
        return None;
    }
    let groups: Vec<&str> = digits.split(separator).collect();
    let (first, rest) = groups.split_first()?;
    let valid = (1..=THOUSANDS_GROUP).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && !rest.is_empty()
        && rest.iter().all(|group| {
            group.len() == THOUSANDS_GROUP && group.bytes().all(|b| b.is_ascii_digit())
        });
    valid.then(|| groups.concat())
}

fn parse_bound(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn split_range(text: &str) -> Option<(Option<i64>, Option<i64>)> {
    let separator = text.find(['-', ':', ','])?;
    let (low, high) = (text.get(..separator)?, text.get(separator + 1..)?);
    let (lo, hi) = (parse_bound(low)?, parse_bound(high)?);
    (lo <= hi).then_some((Some(lo), Some(hi)))
}

fn local_instant(text: &str, time_zone: &TimeZone) -> Option<String> {
    let (date, time) = text.split_once([' ', 'T', 't'])?;
    let mut date_parts = date.split('-');
    let year = date_parts
        .next()
        .filter(|part| part.len() == 4)?
        .parse::<i16>()
        .ok()?;
    let month = date_parts
        .next()
        .filter(|part| part.len() == 2)?
        .parse::<i8>()
        .ok()?;
    let day = date_parts
        .next()
        .filter(|part| part.len() == 2)?
        .parse::<i8>()
        .ok()?;
    let mut time_parts = time.trim().split(':');
    let hour = time_parts
        .next()
        .filter(|part| part.len() == 2)?
        .parse::<i8>()
        .ok()?;
    let minute = time_parts
        .next()
        .filter(|part| part.len() == 2)?
        .parse::<i8>()
        .ok()?;
    let second = match time_parts.next() {
        Some(part) if part.len() == 2 => part.parse::<i8>().ok()?,
        Some(_) => return None,
        None => 0,
    };
    if date_parts.next().is_some() || time_parts.next().is_some() {
        return None;
    }
    let local = DateTime::new(year, month, day, hour, minute, second, 0).ok()?;
    let zoned = local.to_zoned_datetime(time_zone.clone()).ok()?;
    Some(zoned.strftime(INSTANT_FORMAT).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::load_embedded;

    fn converted(outcome: Outcome) -> Converted {
        match outcome {
            Outcome::Converted(converted) => converted,
            other => panic!("expected a conversion, found {other:?}"),
        }
    }

    fn sized(text: &str) -> (u64, String, RuleId, Class) {
        let tables = load_embedded().unwrap();
        let found = converted(size(Written::bare(text), &tables));
        let BoundExpr::Size { v } = found.value else {
            panic!("expected a size");
        };
        (v, found.to, found.rule, found.class)
    }

    fn millis(text: &str) -> Outcome {
        let tables = load_embedded().unwrap();
        duration(Written::quoted(text), &tables)
    }

    fn duration_text(text: &str) -> (u64, String) {
        let found = converted(millis(text));
        let BoundExpr::Dur { v } = found.value else {
            panic!("expected a duration");
        };
        (v, found.to)
    }

    #[test]
    fn rewrites_size_units() {
        assert_eq!(
            sized("10 MB"),
            (10_000_000, "10mb".into(), RuleId::T01, Class::Certain)
        );
        assert_eq!(
            sized("10Mb"),
            (10_000_000, "10mb".into(), RuleId::T01, Class::Certain)
        );
        assert_eq!(
            sized("10 megabytes"),
            (10_000_000, "10mb".into(), RuleId::T01, Class::Certain)
        );
        assert_eq!(
            sized("1.5 GiB"),
            (1_610_612_736, "1.5gib".into(), RuleId::T01, Class::Certain)
        );
        assert_eq!(sized("2 KiB").0, 2_048);
        assert_eq!(sized("3 bytes").0, 3);
        assert_eq!(sized("1 TB").0, 1_000_000_000_000);
    }

    #[test]
    fn reads_single_letter_units_as_binary_with_a_decimal_alternative() {
        let tables = load_embedded().unwrap();
        let found = converted(size(Written::bare("10M"), &tables));
        assert_eq!(
            found.value,
            BoundExpr::Size {
                v: 10 * 1_024 * 1_024
            }
        );
        assert_eq!(
            (found.rule, found.class, found.to.as_str()),
            (RuleId::T02, Class::Probable, "10mib")
        );
        assert_eq!(
            found.alternatives,
            [Alternative {
                value: BoundExpr::Size { v: 10_000_000 },
                text: "10mb".into()
            }]
        );
        assert_eq!(
            found.correction().to_string(),
            "T02  10M → 10mib  size  also: 10mb"
        );
        assert_eq!(sized("64k").0, 65_536);
        assert_eq!(sized("2G").1, "2gib");
    }

    #[test]
    fn leaves_valid_or_unknown_sizes() {
        let tables = load_embedded().unwrap();
        for text in [
            "10mb",
            "10 parsecs",
            "mb",
            "1.2.3 mb",
            "10. mb",
            "0.0001 kb",
            "10 m b",
        ] {
            assert_eq!(
                size(Written::bare(text), &tables),
                Outcome::NotApplicable,
                "{text}"
            );
        }
    }

    #[test]
    fn rewrites_durations() {
        assert_eq!(duration_text("1 hour"), (3_600_000, "1h".into()));
        assert_eq!(duration_text("1hr"), (3_600_000, "1h".into()));
        assert_eq!(duration_text("90 minutes"), (5_400_000, "1h30m".into()));
        assert_eq!(duration_text("2 days"), (172_800_000, "2d".into()));
        assert_eq!(duration_text("1.5h"), (5_400_000, "1h30m".into()));
        assert_eq!(duration_text("PT1H"), (3_600_000, "1h".into()));
        assert_eq!(duration_text("P1W"), (604_800_000, "7d".into()));
        assert_eq!(duration_text("P1DT2H30M"), (95_400_000, "1d2h30m".into()));
        assert_eq!(duration_text("PT0.5S"), (500, "500ms".into()));
        assert_eq!(
            duration_text("1 hour, 30 minutes"),
            (5_400_000, "1h30m".into())
        );
        assert_eq!(
            duration_text("1 hour and 15 min"),
            (4_500_000, "1h15m".into())
        );
        assert_eq!(duration_text("2 weeks"), (1_209_600_000, "14d".into()));
        assert_eq!(duration_text("0 seconds"), (0, "0s".into()));
        assert_eq!(duration_text("250 ms"), (250, "250ms".into()));
    }

    #[test]
    fn refuses_months_and_years() {
        for text in ["2 months", "1 year", "P1M", "P1Y2D"] {
            let Outcome::Uncorrectable(found) = millis(text) else {
                panic!("expected an uncorrectable duration for {text}");
            };
            assert_eq!(found.rule, RuleId::T03);
            assert!(found.problem.starts_with("months and years"));
        }
    }

    #[test]
    fn leaves_canonical_or_unknown_durations() {
        for text in [
            "1h30m",
            "30s",
            "soon",
            "hours",
            "1 fortnight",
            "P",
            "PT",
            "1 2 hours",
            "P1H",
            "1.0001 ms",
        ] {
            assert_eq!(millis(text), Outcome::NotApplicable, "{text}");
        }
    }

    #[test]
    fn rewrites_boolean_words() {
        let tables = load_embedded().unwrap();
        for (text, expected) in [
            ("yes", true),
            ("off", false),
            ("1", true),
            ("0", false),
            ("True", true),
            ("NO", false),
        ] {
            let found = converted(boolean(Written::bare(text), &tables));
            assert_eq!(found.value, BoundExpr::Bool { v: expected }, "{text}");
        }
        let quoted = converted(boolean(Written::quoted("true"), &tables));
        assert_eq!(
            (quoted.from.as_str(), quoted.to.as_str()),
            ("\"true\"", "true")
        );
        assert_eq!(
            boolean(Written::bare("true"), &tables),
            Outcome::NotApplicable
        );
        assert_eq!(
            boolean(Written::bare("maybe"), &tables),
            Outcome::NotApplicable
        );
    }

    #[test]
    fn rewrites_integers_without_changing_the_value() {
        for (text, expected) in [
            ("40.0", 40),
            ("1,000", 1_000),
            ("1 000", 1_000),
            ("-2,500,000", -2_500_000),
            ("12_345", 12_345),
        ] {
            let found = converted(integer(Written::bare(text)));
            assert_eq!(found.value, BoundExpr::Int { v: expected }, "{text}");
            assert_eq!(found.rule, RuleId::T05);
        }
        assert_eq!(converted(integer(Written::quoted("42"))).to, "42");
        for text in [
            "40", "40.5", "1,00", "1,0000", "12,345.6", "x", "1.000", ",100", "1, 000",
        ] {
            assert_eq!(
                integer(Written::bare(text)),
                Outcome::NotApplicable,
                "{text}"
            );
        }
    }

    #[test]
    fn rewrites_ranges() {
        for (text, lo, hi, to) in [
            ("10-20", Some(10), Some(20), "10..20"),
            ("10:20", Some(10), Some(20), "10..20"),
            ("10,20", Some(10), Some(20), "10..20"),
            (">=40", Some(40), None, "40.."),
            ("<= 5", None, Some(5), "..5"),
        ] {
            let found = converted(range(Written::bare(text)));
            assert_eq!(
                (found.value, found.to.as_str()),
                (BoundExpr::Range { lo, hi }, to),
                "{text}"
            );
        }
        for text in ["20-10", ">40", "<4", "10..20", "a-b", "10-", "-5-10", ">=x"] {
            assert_eq!(range(Written::bare(text)), Outcome::NotApplicable, "{text}");
        }
    }

    #[test]
    fn reads_texts_as_regular_expressions_outside_literal_commands() {
        let found = converted(regex(Written::quoted("a/b+"), Some("filter")));
        assert_eq!(
            found.value,
            BoundExpr::Regex {
                v: "a/b+".into(),
                flags: String::new()
            }
        );
        assert_eq!(
            (found.to.as_str(), found.class),
            ("/a\\/b+/", Class::Probable)
        );
        assert_eq!(
            regex(Written::quoted("x"), Some("search")),
            Outcome::NotApplicable
        );
        assert_eq!(
            regex(Written::quoted("x"), Some("replace")),
            Outcome::NotApplicable
        );
        assert_eq!(regex(Written::quoted("x("), None), Outcome::NotApplicable);
        assert_eq!(regex(Written::bare("x"), None), Outcome::NotApplicable);
    }

    #[test]
    fn reads_relative_and_local_instants() {
        let tables = load_embedded().unwrap();
        let warsaw = TimeZone::fixed(jiff::tz::offset(1));
        let yesterday = converted(instant(Written::bare("yesterday"), &tables, &warsaw));
        assert_eq!(
            yesterday.value,
            BoundExpr::Call {
                function: "ago".into(),
                args: vec![BoundExpr::Dur { v: MILLIS_PER_DAY }]
            }
        );
        assert_eq!(
            (yesterday.to.as_str(), yesterday.class),
            ("ago(1d)", Class::Probable)
        );
        let week = converted(instant(Written::quoted("last  week"), &tables, &warsaw));
        assert_eq!(week.to, "ago(7d)");
        let local = converted(instant(
            Written::quoted("2026-11-21 10:00"),
            &tables,
            &warsaw,
        ));
        assert_eq!(
            local.value,
            BoundExpr::Instant {
                v: "2026-11-21T10:00:00+01:00".into()
            }
        );
        assert_eq!(local.to, "\"2026-11-21T10:00:00+01:00\"");
        let utc = converted(instant(
            Written::quoted("2026-11-21T10:00:30"),
            &tables,
            &TimeZone::UTC,
        ));
        assert_eq!(
            utc.value,
            BoundExpr::Instant {
                v: "2026-11-21T10:00:30+00:00".into()
            }
        );
        for text in [
            "2026-11-21",
            "2026-11-21T10:00:00Z",
            "2026-13-01 10:00",
            "tomorrow",
            "2026-11-21 10",
            "2026-11-21 10:00:00:00",
        ] {
            assert_eq!(
                instant(Written::quoted(text), &tables, &warsaw),
                Outcome::NotApplicable,
                "{text}"
            );
        }
    }

    #[test]
    fn dispatches_by_target() {
        let tables = load_embedded().unwrap();
        let context = Context {
            tables: &tables,
            time_zone: &TimeZone::UTC,
            command: Some("write"),
            options: Options::default(),
        };
        let modes = TypeTarget::Enum(["overwrite", "append", "create"].map(String::from).to_vec());
        let mode = converted(correct(Written::bare("truncate"), &modes, &context));
        assert_eq!(
            (mode.value, mode.rule),
            (
                BoundExpr::Enum {
                    v: "overwrite".into()
                },
                RuleId::T09
            )
        );
        assert_eq!(
            correct(Written::bare("sideways"), &modes, &context),
            Outcome::NotApplicable
        );
    }

    #[test]
    fn dispatches_scalar_targets() {
        let tables = load_embedded().unwrap();
        let context = Context {
            tables: &tables,
            time_zone: &TimeZone::UTC,
            command: None,
            options: Options::default(),
        };
        let cases = [
            (Written::bare("yes"), TypeTarget::Bool, true),
            (Written::bare("1,000"), TypeTarget::Int, true),
            (Written::bare("1-2"), TypeTarget::Range, true),
            (Written::quoted("x"), TypeTarget::Regex, true),
            (Written::bare("yesterday"), TypeTarget::Instant, true),
            (Written::quoted("2 days"), TypeTarget::Duration, true),
            (Written::quoted("2 MB"), TypeTarget::Size, true),
            (Written::bare("1,5"), TypeTarget::Number, false),
            (Written::bare("x"), TypeTarget::Text, false),
        ];
        for (written, target, converts) in cases {
            let outcome = correct(written, &target, &context);
            assert_eq!(
                matches!(outcome, Outcome::Converted(_)),
                converts,
                "{target:?}"
            );
        }
    }

    #[test]
    fn formats_durations_with_largest_units_first() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(1), "1ms");
        assert_eq!(
            format_duration(MILLIS_PER_WEEK + MILLIS_PER_SECOND + 5),
            "7d1s5ms"
        );
        assert_eq!(parse_canonical_duration("1h30m"), Some(5_400_000));
        assert_eq!(parse_canonical_duration("1x"), None);
        assert_eq!(parse_canonical_duration(""), None);
    }

    #[test]
    fn leaves_numbers_that_overflow() {
        let tables = load_embedded().unwrap();
        assert_eq!(
            size(Written::bare("99999999999999999999 tb"), &tables),
            Outcome::NotApplicable
        );
        assert_eq!(
            integer(Written::bare("99999999999999999999")),
            Outcome::NotApplicable
        );
        assert_eq!(
            integer(Written::bare("1,000,000,000,000,000,000,000")),
            Outcome::NotApplicable
        );
        assert_eq!(
            range(Written::bare("1-99999999999999999999")),
            Outcome::NotApplicable
        );
        assert_eq!(
            duration(Written::quoted("99999999999999999999 weeks"), &tables),
            Outcome::NotApplicable
        );
    }

    #[test]
    fn leaves_multibyte_literals_untouched() {
        let tables = load_embedded().unwrap();
        assert_eq!(
            size(Written::bare("10 мб"), &tables),
            Outcome::NotApplicable
        );
        assert_eq!(
            instant(
                Written::quoted("2026-11-21 10:00ł"),
                &tables,
                &TimeZone::UTC
            ),
            Outcome::NotApplicable
        );
        assert_eq!(
            duration(Written::quoted("zażółć"), &tables),
            Outcome::NotApplicable
        );
    }

    #[test]
    fn displays_the_literal_as_written() {
        assert_eq!(Written::bare("x").display(), "x");
        assert_eq!(Written::quoted("x y").display(), "\"x y\"");
    }
}
