// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::value::number
// Responsibility:   Bases of integer literals with a prefix.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use nurt_contracts::value::Value;

use super::Syntax;
use super::quote::{PlainKind, yaml_number_kind};

/// Bases of integer literals with a prefix.
const HEX: u32 = 16;
const OCTAL: u32 = 8;
const BINARY: u32 = 2;
const DECIMAL: u32 = 10;
/// Base of the digit groups of YAML 1.1 sexagesimal numbers (`1:30`).
const SEXAGESIMAL: i128 = 60;

/// Value of the number `lexeme` written in `syntax`, or `None` when the text
/// is not a number of that syntax.
///
/// Integers within 64 bits become [`Value::Int`], larger decimal integers
/// [`Value::BigInt`]. A fraction or exponent gives [`Value::Number`] when the
/// shortest form of the binary value is the lexeme itself, otherwise
/// [`Value::Decimal`] with the lexeme (`1.10`, `1e400`, `.inf`, `nan`).
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::{Syntax, parse_number};
///
/// assert_eq!(parse_number("0x1F", Syntax::Toml), Some(Value::Int(31)));
/// assert_eq!(parse_number("1_000", Syntax::Toml), Some(Value::Int(1000)));
/// assert_eq!(parse_number("1:30", Syntax::Yaml11), Some(Value::Int(90)));
/// assert_eq!(parse_number("1.5", Syntax::Json), Some(Value::Number(1.5)));
/// assert_eq!(parse_number("01", Syntax::Json), None);
/// ```
#[must_use]
pub fn parse_number(lexeme: &str, syntax: Syntax) -> Option<Value> {
    match syntax {
        Syntax::Json => json_number(lexeme),
        Syntax::Toml => toml_number(lexeme),
        Syntax::Yaml11 | Syntax::Yaml12 => yaml_number(lexeme, syntax),
        Syntax::Untyped => None,
    }
}

/// Text of a scalar value in `syntax`: `null`, booleans, numbers and dates.
///
/// Returns `None` for text (the caller writes it with the quoting rules of
/// the format), for containers and bytes, for `null` in TOML, which has no
/// null, and for a [`Value::Decimal`] whose lexeme is not a number of the
/// syntax. Floating point numbers keep a fraction or exponent (`2.0`).
///
/// ```
/// use nurt_contracts::value::Value;
/// use nurt_doc::value::{Syntax, render_scalar};
///
/// assert_eq!(render_scalar(&Value::Bool(true), Syntax::Yaml12).as_deref(), Some("true"));
/// assert_eq!(render_scalar(&Value::Number(1e300), Syntax::Yaml11).as_deref(), Some("1.0e+300"));
/// assert_eq!(render_scalar(&Value::Null, Syntax::Toml), None);
/// ```
#[must_use]
pub fn render_scalar(value: &Value, syntax: Syntax) -> Option<String> {
    match value {
        Value::Null => match syntax {
            Syntax::Toml => None,
            Syntax::Untyped => Some(String::new()),
            Syntax::Json | Syntax::Yaml11 | Syntax::Yaml12 => Some("null".to_owned()),
        },
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Int(number) => Some(number.to_string()),
        Value::BigInt(digits) => Some(digits.clone()),
        Value::Number(number) if number.is_finite() => Some(render_float(*number, syntax)),
        Value::Decimal(lexeme) => (syntax == Syntax::Untyped
            || parse_number(lexeme, syntax).is_some())
        .then(|| lexeme.clone()),
        Value::Date(date) => Some(date.text.clone()),
        _ => None,
    }
}

/// Shortest text that reads back as `number`, with a fraction or exponent.
fn render_float(number: f64, syntax: Syntax) -> String {
    let text = format!("{number:?}");
    if syntax != Syntax::Yaml11 || text.contains('.') {
        return text;
    }
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let sign = if exponent.starts_with('-') { "" } else { "+" };
            format!("{mantissa}.0e{sign}{exponent}")
        }
        None => format!("{text}.0"),
    }
}

/// Integer value of the digits, falling back to the decimal text when the
/// number does not fit 64 bits.
fn integer(negative: bool, digits: &str, radix: u32) -> Option<Value> {
    let Some(signed) = u128::from_str_radix(digits, radix)
        .ok()
        .and_then(|magnitude| i128::try_from(magnitude).ok())
    else {
        return big_integer(negative, digits, radix);
    };
    let signed = if negative { -signed } else { signed };
    Some(i64::try_from(signed).map_or_else(|_| Value::BigInt(signed.to_string()), Value::Int))
}

/// Decimal integer that does not fit 128 bits, as its digits.
fn big_integer(negative: bool, digits: &str, radix: u32) -> Option<Value> {
    if radix != DECIMAL || !all_digits(digits, DECIMAL) {
        return None;
    }
    let sign = if negative { "-" } else { "" };
    Some(Value::BigInt(format!(
        "{sign}{}",
        digits.trim_start_matches('0')
    )))
}

/// Value of a lexeme with a fraction or exponent; `text` is the lexeme
/// without underscores in Rust float syntax.
fn float(lexeme: &str, text: &str) -> Option<Value> {
    let number: f64 = text.parse().ok()?;
    if number.is_finite() && (format!("{number:?}") == lexeme) {
        return Some(Value::Number(number));
    }
    Some(Value::Decimal(lexeme.to_owned()))
}

fn split_sign(text: &str) -> (bool, &str) {
    match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

fn all_digits(text: &str, radix: u32) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_digit(radix))
}

fn json_number(lexeme: &str) -> Option<Value> {
    let unsigned = lexeme.strip_prefix('-').unwrap_or(lexeme);
    let (integral, rest) = unsigned.split_at(
        unsigned
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(unsigned.len()),
    );
    if integral.is_empty() || (integral.len() > 1 && integral.starts_with('0')) {
        return None;
    }
    if rest.is_empty() {
        return integer(lexeme.starts_with('-'), integral, DECIMAL);
    }
    let (fraction, exponent) = match rest.find(['e', 'E']) {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let fraction_ok = fraction.is_empty()
        || fraction
            .strip_prefix('.')
            .is_some_and(|digits| all_digits(digits, DECIMAL));
    let exponent_ok = exponent.is_empty() || {
        let digits = &exponent[1..];
        all_digits(digits.trim_start_matches(['+', '-']), DECIMAL)
            && digits.len() - digits.trim_start_matches(['+', '-']).len() <= 1
    };
    (fraction_ok && exponent_ok)
        .then(|| float(lexeme, lexeme))
        .flatten()
}

fn toml_number(lexeme: &str) -> Option<Value> {
    let (negative, unsigned) = split_sign(lexeme);
    if matches!(unsigned, "inf" | "nan") {
        return Some(Value::Decimal(lexeme.to_owned()));
    }
    for (prefix, radix) in [("0x", HEX), ("0o", OCTAL), ("0b", BINARY)] {
        if let Some(digits) = unsigned.strip_prefix(prefix) {
            if lexeme.starts_with(['+', '-']) || !underscores_ok(digits, radix) {
                return None;
            }
            return integer(false, &digits.replace('_', ""), radix);
        }
    }
    let plain = unsigned.replace('_', "");
    let integral_end = unsigned.find(['.', 'e', 'E']).unwrap_or(unsigned.len());
    let integral = &unsigned[..integral_end];
    if !underscores_ok(integral, DECIMAL) || (integral.len() > 1 && integral.starts_with('0')) {
        return None;
    }
    if integral_end == unsigned.len() {
        return integer(negative, &plain, DECIMAL);
    }
    let rest = &unsigned[integral_end..];
    let valid = rest
        .split(['e', 'E'])
        .enumerate()
        .all(|(part, text)| match part {
            0 if text.is_empty() => true,
            0 => text
                .strip_prefix('.')
                .is_some_and(|digits| underscores_ok(digits, DECIMAL)),
            _ => underscores_ok(text.trim_start_matches(['+', '-']), DECIMAL),
        });
    let signed = if negative { format!("-{plain}") } else { plain };
    valid.then(|| float(lexeme, &signed)).flatten()
}

/// Digits of `radix` with single underscores between digits.
fn underscores_ok(text: &str, radix: u32) -> bool {
    !text.is_empty()
        && !text.starts_with('_')
        && !text.ends_with('_')
        && !text.contains("__")
        && text.chars().all(|c| c == '_' || c.is_digit(radix))
}

fn yaml_number(lexeme: &str, syntax: Syntax) -> Option<Value> {
    let kind = yaml_number_kind(lexeme, syntax == Syntax::Yaml11)?;
    let (negative, unsigned) = split_sign(lexeme);
    let plain = unsigned.replace('_', "");
    match kind {
        PlainKind::Int => yaml_integer(negative, &plain, syntax),
        PlainKind::Float if plain.contains(':') => Some(Value::Decimal(lexeme.to_owned())),
        PlainKind::Float
            if plain.starts_with('.') && plain[1..].chars().all(char::is_alphabetic) =>
        {
            Some(Value::Decimal(lexeme.to_owned()))
        }
        PlainKind::Float => {
            let text = if negative { format!("-{plain}") } else { plain };
            float(lexeme, &text)
        }
        _ => None,
    }
}

fn yaml_integer(negative: bool, plain: &str, syntax: Syntax) -> Option<Value> {
    if let Some(digits) = plain.strip_prefix("0x") {
        return integer(negative, digits, HEX);
    }
    if let Some(digits) = plain.strip_prefix("0o") {
        return integer(negative, digits, OCTAL);
    }
    if let Some(digits) = plain.strip_prefix("0b") {
        return integer(negative, digits, BINARY);
    }
    if syntax == Syntax::Yaml11 {
        if plain.contains(':') {
            return sexagesimal(negative, plain);
        }
        if plain.len() > 1 && plain.starts_with('0') {
            return integer(negative, &plain[1..], OCTAL);
        }
    }
    integer(negative, plain, DECIMAL)
}

fn sexagesimal(negative: bool, plain: &str) -> Option<Value> {
    let mut total: i128 = 0;
    for part in plain.split(':') {
        let digit: i128 = part.parse().ok()?;
        total = total.checked_mul(SEXAGESIMAL)?.checked_add(digit)?;
    }
    let signed = if negative { -total } else { total };
    Some(i64::try_from(signed).map_or_else(|_| Value::BigInt(signed.to_string()), Value::Int))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUGE: &str = "123456789012345678901234567890123456789012345";

    #[test]
    fn integers_beyond_128_bits_keep_their_digits() {
        let negative = format!("-{HUGE}");
        for syntax in [Syntax::Json, Syntax::Toml, Syntax::Yaml11, Syntax::Yaml12] {
            assert_eq!(
                parse_number(HUGE, syntax),
                Some(Value::BigInt(HUGE.to_owned())),
                "{syntax:?}"
            );
            assert_eq!(
                parse_number(&negative, syntax),
                Some(Value::BigInt(negative.clone())),
                "{syntax:?}"
            );
        }
    }

    #[test]
    fn integers_between_64_and_128_bits_become_big_integers() {
        assert_eq!(
            parse_number("18446744073709551616", Syntax::Json),
            Some(Value::BigInt("18446744073709551616".to_owned()))
        );
        assert_eq!(
            parse_number("-9223372036854775809", Syntax::Json),
            Some(Value::BigInt("-9223372036854775809".to_owned()))
        );
        assert_eq!(
            parse_number("9223372036854775807", Syntax::Json),
            Some(Value::Int(i64::MAX))
        );
        assert_eq!(
            parse_number("-9223372036854775808", Syntax::Json),
            Some(Value::Int(i64::MIN))
        );
    }

    #[test]
    fn huge_hexadecimal_and_malformed_integers_are_not_numbers() {
        assert_eq!(parse_number(&format!("0x{HUGE}{HUGE}"), Syntax::Toml), None);
        assert_eq!(parse_number("12a", Syntax::Yaml12), None);
        assert_eq!(parse_number("", Syntax::Json), None);
        assert_eq!(parse_number("-", Syntax::Json), None);
    }

    #[test]
    fn json_rejects_leading_zeros_and_bad_exponents() {
        for lexeme in ["01", "1.", ".5", "1e", "1e+-5", "+1", "1.5.2", "0x10"] {
            assert_eq!(parse_number(lexeme, Syntax::Json), None, "{lexeme}");
        }
        assert_eq!(
            parse_number("1.5e3", Syntax::Json),
            Some(Value::Decimal("1.5e3".to_owned()))
        );
        assert_eq!(
            parse_number("1e400", Syntax::Json),
            Some(Value::Decimal("1e400".to_owned()))
        );
    }

    #[test]
    fn yaml_versions_type_octal_and_sexagesimal_numbers() {
        assert_eq!(parse_number("0755", Syntax::Yaml11), Some(Value::Int(493)));
        assert_eq!(parse_number("0755", Syntax::Yaml12), Some(Value::Int(755)));
        assert_eq!(parse_number("09", Syntax::Yaml11), None);
        assert_eq!(parse_number("-1:30", Syntax::Yaml11), Some(Value::Int(-90)));
        assert_eq!(parse_number("1:30", Syntax::Yaml12), None);
        assert_eq!(
            parse_number(".inf", Syntax::Yaml12),
            Some(Value::Decimal(".inf".to_owned()))
        );
    }

    #[test]
    fn toml_numbers_follow_the_underscore_rules() {
        assert_eq!(parse_number("1_000", Syntax::Toml), Some(Value::Int(1000)));
        assert_eq!(parse_number("1__0", Syntax::Toml), None);
        assert_eq!(parse_number("_1", Syntax::Toml), None);
        assert_eq!(parse_number("-0x1F", Syntax::Toml), None);
        assert_eq!(parse_number("0b101", Syntax::Toml), Some(Value::Int(5)));
        assert_eq!(parse_number("007", Syntax::Toml), None);
        assert_eq!(
            parse_number("-inf", Syntax::Toml),
            Some(Value::Decimal("-inf".to_owned()))
        );
    }

    #[test]
    fn rendering_keeps_a_fraction_or_exponent() {
        assert_eq!(
            render_scalar(&Value::Number(2.0), Syntax::Yaml11).as_deref(),
            Some("2.0")
        );
        assert_eq!(
            render_scalar(&Value::Number(1e16), Syntax::Yaml11).as_deref(),
            Some("1.0e+16")
        );
        assert_eq!(
            render_scalar(&Value::Number(1e-7), Syntax::Yaml11).as_deref(),
            Some("1.0e-7")
        );
        assert_eq!(render_scalar(&Value::Number(f64::NAN), Syntax::Json), None);
        assert_eq!(
            render_scalar(&Value::Decimal("1.10".to_owned()), Syntax::Json).as_deref(),
            Some("1.10")
        );
        assert_eq!(
            render_scalar(&Value::Decimal("not a number".to_owned()), Syntax::Json),
            None
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
