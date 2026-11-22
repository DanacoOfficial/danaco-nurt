// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::borrow::Cow;

use nurt_contracts::ast::Expr;

use super::error::{Fault, LexError};
use super::scan::{self, Lexeme, Rejected, Scanned};
use super::token::{MAX_SAFE_INTEGER, TokenKind};

/// [`MAX_SAFE_INTEGER`] as an unsigned value.
const MAX_SAFE: u64 = MAX_SAFE_INTEGER.unsigned_abs();

/// [`MAX_SAFE_INTEGER`] as a float; exactly representable.
const MAX_SAFE_FLOAT: f64 = 9_007_199_254_740_991.0;

/// Size units and their multipliers in bytes.
const SIZE_UNITS: [(&str, u64); 9] = [
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

/// Duration units and their lengths in milliseconds, from the largest
/// printed unit down; weeks are read but never printed.
const DURATION_UNITS: [(&str, u64); 6] = [
    ("w", 604_800_000),
    ("d", 86_400_000),
    ("h", 3_600_000),
    ("m", 60_000),
    ("s", 1_000),
    ("ms", 1),
];

/// Number of units of [`DURATION_UNITS`] that are never printed.
const UNPRINTED_DURATION_UNITS: usize = 1;

/// Unit names written in other conventions and the unit of the language.
const UNIT_SYNONYMS: [(&str, &str); 20] = [
    ("byte", "b"),
    ("bytes", "b"),
    ("msec", "ms"),
    ("millis", "ms"),
    ("sec", "s"),
    ("secs", "s"),
    ("second", "s"),
    ("seconds", "s"),
    ("min", "m"),
    ("mins", "m"),
    ("minute", "m"),
    ("minutes", "m"),
    ("hr", "h"),
    ("hrs", "h"),
    ("hour", "h"),
    ("hours", "h"),
    ("day", "d"),
    ("days", "d"),
    ("week", "w"),
    ("weeks", "w"),
];

/// Largest number of significant fraction digits of a size or a fractional
/// duration. The fraction times the largest multiplier, `2^40`, stays below
/// `2^128`.
const MAX_FRACTION_DIGITS: u32 = 26;

const DECIMAL_RADIX: u32 = 10;
const HEX_RADIX: u32 = 16;

/// Value of a numeric token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Numeric {
    Int(u64),
    Number(f64),
    Size(u64),
    Duration(u64),
}

impl Numeric {
    const fn kind(self) -> TokenKind {
        match self {
            Self::Int(_) => TokenKind::Int,
            Self::Number(_) => TokenKind::Number,
            Self::Size(_) => TokenKind::Size,
            Self::Duration(_) => TokenKind::Duration,
        }
    }
}

/// Numeric token: its end and value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NumericToken {
    end: usize,
    value: Numeric,
}

/// Digits of a decimal number: `int` or `int.fraction`.
#[derive(Debug, Clone, Copy)]
struct Numeral {
    start: usize,
    int_end: usize,
    end: usize,
}

impl Numeral {
    const fn has_fraction(&self) -> bool {
        self.end > self.int_end
    }

    fn text<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.end).unwrap_or_default()
    }

    fn int_digits<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.int_end).unwrap_or_default()
    }

    fn fraction_digits<'a>(&self, text: &'a str) -> &'a str {
        if self.has_fraction() {
            text.get(self.int_end + 1..self.end).unwrap_or_default()
        } else {
            ""
        }
    }
}

/// Scans the numeric token that starts with the digit at `start`.
pub(crate) fn scan_number(text: &str, start: usize) -> Scanned {
    read(text, start).map(|token| Lexeme::new(token.end, token.value.kind()))
}

fn read(text: &str, start: usize) -> Result<NumericToken, Box<Rejected>> {
    let bytes = text.as_bytes();
    if bytes.get(start) == Some(&b'0') {
        match bytes.get(start + 1) {
            Some(b'x') => return read_hex(text, start),
            Some(b'X') => return Err(scan::reject(upper_hex_prefix(text, start))),
            _ => {}
        }
    }
    let int_end = digits_end(bytes, start);
    let end = fraction_end(bytes, int_end).unwrap_or(int_end);
    let numeral = Numeral {
        start,
        int_end,
        end,
    };
    let unit_end = letters_end(bytes, end);
    if unit_end == end {
        return plain(text, numeral).map_err(scan::reject);
    }
    let unit = text.get(end..unit_end).unwrap_or_default();
    if let Some(exponent_end) = exponent_end(bytes, unit, unit_end) {
        return Err(scan::reject(exponent(text, start, exponent_end)));
    }
    if let Some(multiplier) = size_multiplier(unit) {
        return read_size(text, numeral, unit_end, multiplier);
    }
    if duration_length(unit).is_some() {
        if numeral.has_fraction() {
            return Err(fractional_duration(text, numeral, unit_end));
        }
        return read_duration(text, start);
    }
    Err(unknown_unit(text, numeral, unit_end))
}

fn digits_end(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b'_')
        .count()
}

fn hex_digits_end(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit() || **byte == b'_')
        .count()
}

fn letters_end(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count()
}

fn word_end(bytes: &[u8], from: usize) -> usize {
    from + bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .count()
}

fn is_digit_at(bytes: &[u8], index: usize) -> bool {
    bytes.get(index).is_some_and(u8::is_ascii_digit)
}

/// End of the fraction `.digits` after the integer part, if one follows;
/// `1.` and `1..2` have none.
fn fraction_end(bytes: &[u8], int_end: usize) -> Option<usize> {
    (bytes.get(int_end) == Some(&b'.') && is_digit_at(bytes, int_end + 1))
        .then(|| digits_end(bytes, int_end + 1))
}

fn exponent_end(bytes: &[u8], unit: &str, unit_end: usize) -> Option<usize> {
    if unit != "e" && unit != "E" {
        return None;
    }
    let digits_start = match bytes.get(unit_end) {
        Some(b'+' | b'-') => unit_end + 1,
        _ => unit_end,
    };
    is_digit_at(bytes, digits_start).then(|| digits_end(bytes, digits_start))
}

fn size_multiplier(unit: &str) -> Option<u64> {
    SIZE_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|&(_, multiplier)| multiplier)
}

fn duration_length(unit: &str) -> Option<u64> {
    DURATION_UNITS
        .iter()
        .find(|(name, _)| *name == unit)
        .map(|&(_, length)| length)
}

fn radix_value(digits: &str, radix: u32) -> Option<u64> {
    digits
        .chars()
        .filter(|&character| character != '_')
        .try_fold(0_u64, |value, character| {
            let digit = character.to_digit(radix)?;
            value
                .checked_mul(u64::from(radix))?
                .checked_add(u64::from(digit))
        })
}

fn safe(value: Option<u64>) -> Option<u64> {
    value.filter(|&value| value <= MAX_SAFE)
}

fn without_underscores(digits: &str) -> Cow<'_, str> {
    if digits.contains('_') {
        Cow::Owned(digits.replace('_', ""))
    } else {
        Cow::Borrowed(digits)
    }
}

fn too_large(text: &str, start: usize, end: usize, what: &str) -> Fault {
    let found = text.get(start..end).unwrap_or_default();
    Fault::lexical(
        start..end,
        "number_too_large",
        format!("{what} `{found}` exceeds {MAX_SAFE_INTEGER} (2^53 - 1); write a smaller value"),
        "numbers",
    )
}

fn plain(text: &str, numeral: Numeral) -> Result<NumericToken, Fault> {
    let end = numeral.end;
    if numeral.has_fraction() {
        let value = without_underscores(numeral.text(text))
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite());
        return value
            .map(|value| NumericToken {
                end,
                value: Numeric::Number(value),
            })
            .ok_or_else(|| too_large(text, numeral.start, end, "number"));
    }
    safe(radix_value(numeral.int_digits(text), DECIMAL_RADIX))
        .map(|value| NumericToken {
            end,
            value: Numeric::Int(value),
        })
        .ok_or_else(|| too_large(text, numeral.start, end, "integer"))
}

/// Exact value of `numeral × multiplier` as a whole number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scaled {
    Whole(u64),
    Fractional,
    TooPrecise,
    TooLarge,
}

fn scale(text: &str, numeral: Numeral, multiplier: u64) -> Scaled {
    let Some(int) = safe(radix_value(numeral.int_digits(text), DECIMAL_RADIX)) else {
        return Scaled::TooLarge;
    };
    let digits: Vec<u32> = numeral
        .fraction_digits(text)
        .chars()
        .filter_map(|character| character.to_digit(DECIMAL_RADIX))
        .collect();
    let significant = digits
        .iter()
        .rposition(|&digit| digit != 0)
        .map_or(0, |last| last + 1);
    let Some(exponent) = u32::try_from(significant)
        .ok()
        .filter(|&exponent| exponent <= MAX_FRACTION_DIGITS)
    else {
        return Scaled::TooPrecise;
    };
    let fraction = digits
        .iter()
        .take(significant)
        .fold(0_u128, |value, &digit| {
            value * u128::from(DECIMAL_RADIX) + u128::from(digit)
        });
    let denominator = u128::from(DECIMAL_RADIX).pow(exponent);
    let part = fraction * u128::from(multiplier);
    if part % denominator != 0 {
        return Scaled::Fractional;
    }
    let total = u128::from(int) * u128::from(multiplier) + part / denominator;
    u64::try_from(total)
        .ok()
        .filter(|&total| total <= MAX_SAFE)
        .map_or(Scaled::TooLarge, Scaled::Whole)
}

fn read_size(
    text: &str,
    numeral: Numeral,
    unit_end: usize,
    multiplier: u64,
) -> Result<NumericToken, Box<Rejected>> {
    let bytes = text.as_bytes();
    let start = numeral.start;
    if bytes
        .get(unit_end)
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
    {
        let end = word_end(bytes, unit_end);
        let found = text.get(start..end).unwrap_or_default();
        return Err(scan::reject(Fault::lexical(
            start..end,
            "size_suffix",
            format!("`{found}` has digits after the unit; separate them with a space"),
            "numbers",
        )));
    }
    let found = text.get(start..unit_end).unwrap_or_default();
    let fault = |problem, message: String| {
        scan::reject(Fault::lexical(start..unit_end, problem, message, "numbers"))
    };
    match scale(text, numeral, multiplier) {
        Scaled::Whole(value) => Ok(NumericToken {
            end: unit_end,
            value: Numeric::Size(value),
        }),
        Scaled::Fractional => Err(fault(
            "size_not_whole",
            format!("size `{found}` is not a whole number of bytes; use a smaller unit"),
        )),
        Scaled::TooPrecise => Err(fault(
            "size_precision",
            format!(
                "size `{found}` has more than {MAX_FRACTION_DIGITS} significant fraction digits; use a smaller unit"
            ),
        )),
        Scaled::TooLarge => Err(scan::reject(too_large(text, start, unit_end, "size"))),
    }
}

fn read_duration(text: &str, start: usize) -> Result<NumericToken, Box<Rejected>> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut total = 0_u64;
    loop {
        let digits = digits_end(bytes, index);
        let unit_end = letters_end(bytes, digits);
        let unit = text.get(digits..unit_end).unwrap_or_default();
        let Some(length) = duration_length(unit) else {
            return Err(scan::reject(duration_part_fault(text, start, unit)));
        };
        let value = text.get(index..digits).unwrap_or_default();
        let sum = radix_value(value, DECIMAL_RADIX)
            .and_then(|value| value.checked_mul(length))
            .and_then(|part| part.checked_add(total));
        total = match safe(sum) {
            Some(total) => total,
            None => {
                let end = word_end(bytes, unit_end);
                return Err(scan::reject(too_large(text, start, end, "duration")));
            }
        };
        index = unit_end;
        if !is_digit_at(bytes, index) {
            return Ok(NumericToken {
                end: index,
                value: Numeric::Duration(total),
            });
        }
    }
}

fn duration_part_fault(text: &str, start: usize, unit: &str) -> Fault {
    let end = word_end(text.as_bytes(), start);
    let found = text.get(start..end).unwrap_or_default();
    let (problem, message) = if unit.is_empty() {
        (
            "duration_without_unit",
            format!(
                "`{found}` ends with digits without a unit; write a unit after every number, as in `1m30s`"
            ),
        )
    } else if size_multiplier(unit).is_some() {
        (
            "mixed_units",
            format!("`{found}` mixes a duration with the size unit `{unit}`"),
        )
    } else {
        (
            "unknown_unit",
            format!(
                "unknown duration unit `{unit}` in `{found}`; the units are ms, s, m, h, d and w"
            ),
        )
    };
    Fault::lexical(start..end, problem, message, "numbers")
}

fn fractional_duration(text: &str, numeral: Numeral, unit_end: usize) -> Box<Rejected> {
    let bytes = text.as_bytes();
    let end = word_end(bytes, unit_end);
    let found = text.get(numeral.start..end).unwrap_or_default();
    let unit = text.get(numeral.end..unit_end).unwrap_or_default();
    let fix = match (end == unit_end, duration_length(unit)) {
        (true, Some(length)) => match scale(text, numeral, length) {
            Scaled::Whole(milliseconds) => valid_fix(format_duration(milliseconds)),
            _ => None,
        },
        _ => None,
    };
    let fault = Fault::lexical(
        numeral.start..end,
        "fractional_duration",
        format!("duration `{found}` has a fraction; combine whole units, as in `1h30m`"),
        "numbers",
    )
    .with_optional_fix(fix);
    recover_number(text, numeral, fault)
}

fn unknown_unit(text: &str, numeral: Numeral, unit_end: usize) -> Box<Rejected> {
    let bytes = text.as_bytes();
    let end = word_end(bytes, unit_end);
    let unit = text.get(numeral.end..unit_end).unwrap_or_default();
    let number = numeral.text(text);
    let lower = unit.to_ascii_lowercase();
    let known = size_multiplier(&lower).is_some() || duration_length(&lower).is_some();
    let synonym = UNIT_SYNONYMS
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|&(_, canonical)| canonical);
    let (problem, message, fix) = if known && lower != unit {
        let fix = (lower.len() > 1).then(|| format!("{number}{lower}"));
        (
            "uppercase_unit",
            format!("unit `{unit}` must be written in lower case: `{lower}`"),
            fix,
        )
    } else if let Some(canonical) = synonym {
        (
            "unknown_unit",
            format!("unknown unit `{unit}`; write `{canonical}`"),
            Some(format!("{number}{canonical}")),
        )
    } else {
        (
            "letters_after_number",
            format!(
                "letters `{unit}` directly follow a number; separate them with a space or use a unit of size (b, kb, mib, …) or duration (ms, s, m, h, d, w)"
            ),
            None,
        )
    };
    let fault = Fault::lexical(numeral.start..end, problem, message, "numbers")
        .with_optional_fix(fix.filter(|_| end == unit_end).and_then(valid_fix));
    recover_number(text, numeral, fault)
}

/// Rejection whose tolerant token is the number before the letters, which
/// the lexer returns next as a loose word.
fn recover_number(text: &str, numeral: Numeral, fault: Fault) -> Box<Rejected> {
    match plain(text, numeral) {
        Ok(token) => Box::new(Rejected {
            fault,
            recovery: Some(Lexeme::new(token.end, token.value.kind())),
            glued_word: true,
        }),
        Err(_) => scan::reject(fault),
    }
}

fn exponent(text: &str, start: usize, exponent_end: usize) -> Fault {
    let end = word_end(text.as_bytes(), exponent_end);
    let found = text.get(start..end).unwrap_or_default();
    let fix = (end == exponent_end)
        .then(|| without_underscores(found).parse::<f64>().ok())
        .flatten()
        .and_then(format_decimal)
        .and_then(valid_fix);
    Fault::lexical(
        start..end,
        "exponent",
        format!("`{found}` uses an exponent; write the number with all its digits"),
        "numbers",
    )
    .with_optional_fix(fix)
}

fn read_hex(text: &str, start: usize) -> Result<NumericToken, Box<Rejected>> {
    let bytes = text.as_bytes();
    let digits_start = start + 2;
    if !bytes.get(digits_start).is_some_and(u8::is_ascii_hexdigit) {
        let end = word_end(bytes, digits_start);
        return Err(scan::reject(Fault::lexical(
            start..end,
            "hex_without_digits",
            "write at least one hex digit directly after `0x`",
            "numbers",
        )));
    }
    let end = hex_digits_end(bytes, digits_start);
    let value = safe(radix_value(
        text.get(digits_start..end).unwrap_or_default(),
        HEX_RADIX,
    ));
    if bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
        return Err(hex_letters(text, start, end, value));
    }
    if let Some(fraction) = fraction_end(bytes, end) {
        return Err(scan::reject(Fault::lexical(
            start..fraction,
            "hex_fraction",
            "a hexadecimal number has no fraction; write the number in decimal digits",
            "numbers",
        )));
    }
    match value {
        Some(value) => Ok(NumericToken {
            end,
            value: Numeric::Int(value),
        }),
        None => Err(scan::reject(too_large(text, start, end, "integer"))),
    }
}

fn hex_letters(text: &str, start: usize, end: usize, value: Option<u64>) -> Box<Rejected> {
    let bytes = text.as_bytes();
    let unit_end = letters_end(bytes, end);
    let word = word_end(bytes, end);
    let unit = text.get(end..unit_end).unwrap_or_default();
    if size_multiplier(unit).is_some() || duration_length(unit).is_some() {
        let fix = value
            .filter(|_| word == unit_end)
            .and_then(|value| valid_fix(format!("{value}{unit}")));
        return scan::reject(
            Fault::lexical(
                start..word,
                "hex_with_unit",
                "a unit cannot follow a hexadecimal number; write the number in decimal digits",
                "numbers",
            )
            .with_optional_fix(fix),
        );
    }
    let fault = Fault::lexical(
        start..word,
        "letters_after_number",
        format!(
            "letters `{}` directly follow a number; separate them with a space",
            text.get(end..word).unwrap_or_default()
        ),
        "numbers",
    );
    match value {
        Some(_) => Box::new(Rejected {
            fault,
            recovery: Some(Lexeme::new(end, TokenKind::Int)),
            glued_word: true,
        }),
        None => scan::reject(fault),
    }
}

fn upper_hex_prefix(text: &str, start: usize) -> Fault {
    let end = word_end(text.as_bytes(), start);
    let found = text.get(start..end).unwrap_or_default();
    let fix = found
        .get(2..)
        .and_then(|digits| valid_fix(format!("0x{digits}")));
    Fault::lexical(
        start..end,
        "hex_prefix",
        "write the hexadecimal prefix in lower case: `0x`",
        "numbers",
    )
    .with_optional_fix(fix)
}

/// `candidate` when it is exactly one valid numeric token.
fn valid_fix(candidate: String) -> Option<String> {
    matches!(read(&candidate, 0), Ok(token) if token.end == candidate.len()).then_some(candidate)
}

/// Decimal form of a finite value without an exponent.
fn format_decimal(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if value.fract() != 0.0 {
        return Some(format!("{value}"));
    }
    if value <= MAX_SAFE_FLOAT {
        Some(format!("{value:.0}"))
    } else {
        Some(format!("{value:.1}"))
    }
}

/// Canonical duration: units `d h m s ms` in this order without repetition,
/// greedy from the largest; `0s` for zero.
pub(crate) fn format_duration(milliseconds: u64) -> String {
    if milliseconds == 0 {
        return "0s".to_owned();
    }
    let mut rest = milliseconds;
    let mut text = String::new();
    for &(unit, length) in DURATION_UNITS.iter().skip(UNPRINTED_DURATION_UNITS) {
        let count = rest / length;
        if count > 0 {
            text.push_str(&count.to_string());
            text.push_str(unit);
            rest %= length;
        }
    }
    text
}

fn not_literal(src: &str, what: &str) -> LexError {
    Fault::lexical(
        0..src.len(),
        "not_a_literal",
        format!("`{src}` is not a single {what} literal"),
        "numbers",
    )
    .into_standalone(src)
}

fn read_whole(src: &str, what: &str) -> Result<Numeric, LexError> {
    if !src.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return Err(not_literal(src, what));
    }
    match read(src, 0) {
        Ok(token) if token.end == src.len() => Ok(token.value),
        Ok(_) => Err(not_literal(src, what)),
        Err(rejected) => Err(rejected.fault.into_standalone(src)),
    }
}

/// Decodes an integer literal: decimal digits or `0x` and hex digits, with
/// optional `_` separators.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one integer literal or the
/// value exceeds [`MAX_SAFE_INTEGER`]. Positions are relative to `src`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::parse_int;
///
/// assert_eq!(parse_int("1_000")?, Expr::Int { v: 1000, span: None });
/// assert_eq!(parse_int("0x1F")?, Expr::Int { v: 31, span: None });
/// assert!(parse_int("9007199254740992").is_err());
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn parse_int(src: &str) -> Result<Expr, LexError> {
    match read_whole(src, "integer")? {
        Numeric::Int(value) => i64::try_from(value)
            .map(|v| Expr::Int { v, span: None })
            .map_err(|_| not_literal(src, "integer")),
        _ => Err(not_literal(src, "integer")),
    }
}

/// Decodes a number with a fraction, `INT.INT`.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one number literal with a
/// fraction. Positions are relative to `src`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::parse_number;
///
/// assert_eq!(parse_number("3.5")?, Expr::Num { v: 3.5, span: None });
/// assert!(parse_number("1e5").is_err());
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn parse_number(src: &str) -> Result<Expr, LexError> {
    match read_whole(src, "number")? {
        Numeric::Number(v) => Ok(Expr::Num { v, span: None }),
        _ => Err(not_literal(src, "number")),
    }
}

/// Decodes a size literal into a `size` node in bytes.
///
/// # Errors
///
/// Returns the errors of [`size_bytes`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::parse_size;
///
/// assert_eq!(parse_size("1.5kb")?, Expr::Size { v: 1_500, span: None });
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn parse_size(src: &str) -> Result<Expr, LexError> {
    size_bytes(src).map(|v| Expr::Size { v, span: None })
}

/// Value of a size literal in bytes, computed exactly: decimal units are
/// powers of 1000, binary units powers of 1024.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one size literal, when the
/// value is not a whole number of bytes or when it exceeds
/// [`MAX_SAFE_INTEGER`]. Positions are relative to `src`.
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::size_bytes;
///
/// assert_eq!(size_bytes("1.5gib")?, 1_610_612_736);
/// assert!(size_bytes("0.5b").is_err());
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn size_bytes(src: &str) -> Result<u64, LexError> {
    match read_whole(src, "size")? {
        Numeric::Size(value) => Ok(value),
        _ => Err(not_literal(src, "size")),
    }
}

/// Decodes a duration literal into a `dur` node in milliseconds.
///
/// # Errors
///
/// Returns the errors of [`duration_ms`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::lex::parse_duration;
///
/// assert_eq!(parse_duration("250ms")?, Expr::Dur { v: 250, span: None });
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn parse_duration(src: &str) -> Result<Expr, LexError> {
    duration_ms(src).map(|v| Expr::Dur { v, span: None })
}

/// Value of a duration literal in milliseconds: the sum of its parts.
///
/// # Errors
///
/// Returns `E_LEXICAL` when `src` is not exactly one duration literal or the
/// value exceeds [`MAX_SAFE_INTEGER`]. Positions are relative to `src`.
///
/// # Examples
///
/// ```
/// use nurt_syntax::lex::duration_ms;
///
/// assert_eq!(duration_ms("1h30m")?, 5_400_000);
/// assert!(duration_ms("1.5h").is_err());
/// # Ok::<(), nurt_syntax::lex::LexError>(())
/// ```
pub fn duration_ms(src: &str) -> Result<u64, LexError> {
    match read_whole(src, "duration")? {
        Numeric::Duration(value) => Ok(value),
        _ => Err(not_literal(src, "duration")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Numeric, format_decimal, format_duration, read};

    fn value(src: &str) -> Option<Numeric> {
        read(src, 0)
            .ok()
            .filter(|token| token.end == src.len())
            .map(|token| token.value)
    }

    #[test]
    fn distinguishes_minutes_milliseconds_and_megabytes() {
        assert_eq!(value("5m"), Some(Numeric::Duration(300_000)));
        assert_eq!(value("5ms"), Some(Numeric::Duration(5)));
        assert_eq!(value("5mb"), Some(Numeric::Size(5_000_000)));
    }

    #[test]
    fn computes_sizes_exactly() {
        assert_eq!(value("1.5gib"), Some(Numeric::Size(1_610_612_736)));
        assert_eq!(value("0.5kib"), Some(Numeric::Size(512)));
        assert_eq!(
            value("1.000000000000000000000000000000kb"),
            Some(Numeric::Size(1_000))
        );
        assert_eq!(value("0.0001kb"), None);
    }

    #[test]
    fn formats_durations_canonically() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(5_400_000), "1h30m");
        assert_eq!(format_duration(604_800_000), "7d");
        assert_eq!(format_duration(1_001), "1s1ms");
    }

    #[test]
    fn formats_decimals_without_exponent() {
        assert_eq!(format_decimal(100_000.0).as_deref(), Some("100000"));
        assert_eq!(format_decimal(0.001).as_deref(), Some("0.001"));
        assert_eq!(
            format_decimal(1e20).as_deref(),
            Some("100000000000000000000.0")
        );
        assert_eq!(format_decimal(f64::INFINITY), None);
    }
}
