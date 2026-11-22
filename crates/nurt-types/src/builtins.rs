// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Built-in functions of the expression language.

use std::cmp::Ordering;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Bytes, Range, Regex, Value};
use serde_json::json;

use crate::coerce::{ValueType, coerce_in};
use crate::compare::{comparable, compare, equal};
use crate::env::EvalEnv;
use crate::errors::{TEXT_TYPES, arity, invalid_argument, invalid_value, overflow, type_error};
use crate::pattern::RegexCache;
use crate::units::parse_instant;
use crate::{idna, path};

/// Names of the built-in functions, in the order of the language
/// reference.
pub const FUNCTIONS: &[&str] = &[
    "lower",
    "upper",
    "trim",
    "len",
    "abs",
    "round",
    "min",
    "max",
    "contains",
    "starts_with",
    "ends_with",
    "replace_text",
    "regex",
    "time",
    "now",
    "ago",
    "from_now",
    "parent",
    "join_path",
    "base64_encode",
    "base64_decode",
    "idna_encode",
    "idna_decode",
    "hex",
    "uuid",
    "random",
];

/// Largest number of fraction digits accepted by `round`.
const MAX_ROUND_DIGITS: i64 = 15;
/// 2^63 as a float, the first float above every `i64`.
const I64_LIMIT: f64 = 9_223_372_036_854_775_808.0;
/// Bits of a float mantissa, the precision of `random()`.
const RANDOM_BITS: u32 = 53;
/// 2^-53: scales 53 random bits to `[0, 1)`.
const RANDOM_SCALE: f64 = 1.0 / 9_007_199_254_740_992.0;
const UUID_BYTES: usize = 16;
const BITS_PER_U64: u32 = 64;

/// Evaluated arguments of a call, with the arguments written as text
/// literals marked: only those convert implicitly to the parameter type.
pub(crate) struct Args {
    values: Vec<Value>,
    literal_text: Vec<bool>,
}

impl Args {
    pub(crate) const fn new(values: Vec<Value>, literal_text: Vec<bool>) -> Self {
        Self {
            values,
            literal_text,
        }
    }

    fn get(&self, index: usize) -> &Value {
        self.values.get(index).unwrap_or(&Value::Null)
    }

    fn exactly(&self, function: &str, count: usize) -> Result<()> {
        if self.values.len() == count {
            return Ok(());
        }
        let expected = match count {
            0 => "no arguments".to_owned(),
            1 => "1 argument".to_owned(),
            _ => format!("{count} arguments"),
        };
        Err(arity(function, &expected, self.values.len()))
    }

    fn between(&self, function: &str, min: usize, max: usize) -> Result<()> {
        if (min..=max).contains(&self.values.len()) {
            Ok(())
        } else {
            Err(arity(
                function,
                &format!("{min} to {max} arguments"),
                self.values.len(),
            ))
        }
    }

    fn at_least(&self, function: &str, min: usize) -> Result<()> {
        if self.values.len() >= min {
            Ok(())
        } else {
            Err(arity(
                function,
                &format!("at least {min} arguments"),
                self.values.len(),
            ))
        }
    }

    /// The argument, converted to `target` when it was written as a text
    /// literal.
    fn typed(&self, index: usize, target: ValueType, env: &dyn EvalEnv) -> Result<Value> {
        let value = self.get(index).clone();
        if self.literal_text.get(index).copied().unwrap_or(false) {
            coerce_in(value, target, &env.tz())
        } else {
            Ok(value)
        }
    }
}

/// What a built-in function may use besides its arguments.
pub(crate) struct CallCtx<'a> {
    pub(crate) env: &'a dyn EvalEnv,
    pub(crate) regexes: &'a RegexCache,
}

/// Calls the built-in function `name`.
pub(crate) fn call(name: &str, args: &Args, ctx: &CallCtx<'_>) -> Result<Value> {
    match name {
        "lower" => text_function(name, args, str::to_lowercase),
        "upper" => text_function(name, args, str::to_uppercase),
        "trim" => text_function(name, args, |text| text.trim().to_owned()),
        "len" => len(args),
        "abs" => abs(args),
        "round" => round(args),
        "min" => extreme(name, args, Ordering::Less),
        "max" => extreme(name, args, Ordering::Greater),
        "contains" => contains(args),
        "starts_with" => affix(name, args, |text, affix| text.starts_with(affix)),
        "ends_with" => affix(name, args, |text, affix| text.ends_with(affix)),
        "replace_text" => replace_text(args),
        "regex" => regex(args, ctx.regexes),
        "time" => time(args, ctx.env),
        "now" => args
            .exactly(name, 0)
            .map(|()| Value::Instant(ctx.env.now())),
        "ago" => shifted_now(name, args, ctx.env, true),
        "from_now" => shifted_now(name, args, ctx.env, false),
        "parent" => parent(args, ctx.env),
        "join_path" => join_path(args, ctx.env),
        "base64_encode" => base64_encode(args),
        "base64_decode" => base64_decode(args),
        "idna_encode" => idna_function(name, args, idna::to_ascii),
        "idna_decode" => idna_function(name, args, idna::to_unicode),
        "hex" => hex(args),
        "uuid" => uuid(args, ctx.env),
        "random" => random(args, ctx.env),
        _ => Err(unknown_function(name)),
    }
}

fn unknown_function(name: &str) -> Error {
    Error::new(ErrorCode::UnknownName, format!("unknown function `{name}`"))
        .with_found(name)
        .with_problem("unknown_function")
        .with_expected(json!(FUNCTIONS))
        .with_help("functions")
}

fn context(function: &str) -> String {
    format!("`{function}`")
}

fn text_of<'v>(function: &str, value: &'v Value) -> Result<&'v str> {
    value
        .as_str()
        .ok_or_else(|| type_error(&context(function), TEXT_TYPES, value))
}

/// Applies `map` to the text of a text-like value, keeping its type.
fn map_text(function: &str, value: &Value, map: impl Fn(&str) -> String) -> Result<Value> {
    Ok(match value {
        Value::Null => Value::Null,
        Value::Text(text) => Value::Text(map(text)),
        Value::Path(text) => Value::Path(map(text)),
        Value::Glob(text) => Value::Glob(map(text)),
        Value::Key(text) => Value::Key(map(text)),
        Value::Enum(text) => Value::Enum(map(text)),
        other => return Err(type_error(&context(function), TEXT_TYPES, other)),
    })
}

fn text_function(function: &str, args: &Args, map: impl Fn(&str) -> String) -> Result<Value> {
    args.exactly(function, 1)?;
    map_text(function, args.get(0), map)
}

fn len(args: &Args) -> Result<Value> {
    args.exactly("len", 1)?;
    let count = match args.get(0) {
        Value::Null => return Ok(Value::Null),
        Value::List(items) => items.len(),
        Value::Map(map) => map.len(),
        Value::Record(record) => record.len(),
        Value::Bytes(bytes) => bytes.as_slice().len(),
        other => match other.as_str() {
            Some(text) => text.chars().count(),
            None => {
                return Err(type_error(
                    &context("len"),
                    &["text", "list", "map", "record", "bytes"],
                    other,
                ));
            }
        },
    };
    i64::try_from(count)
        .map(Value::Int)
        .map_err(|_| overflow("`len`", "int"))
}

fn abs(args: &Args) -> Result<Value> {
    args.exactly("abs", 1)?;
    match args.get(0) {
        Value::Null => Ok(Value::Null),
        Value::Int(int) => int
            .checked_abs()
            .map(Value::Int)
            .ok_or_else(|| overflow("`abs`", "int")),
        Value::Number(float) => Ok(Value::Number(float.abs())),
        Value::Duration(duration) => duration
            .as_millis()
            .checked_abs()
            .map(|millis| Value::Duration(nurt_contracts::value::Duration::from_millis(millis)))
            .ok_or_else(|| overflow("`abs`", "duration")),
        Value::BigInt(text) => Ok(Value::BigInt(text.trim_start_matches('-').to_owned())),
        Value::Decimal(text) => Ok(Value::Decimal(text.trim_start_matches('-').to_owned())),
        other => Err(type_error(
            &context("abs"),
            &["int", "number", "duration"],
            other,
        )),
    }
}

fn round(args: &Args) -> Result<Value> {
    args.between("round", 1, 2)?;
    let digits = match args.values.get(1) {
        None => None,
        Some(Value::Int(digits)) if (0..=MAX_ROUND_DIGITS).contains(digits) => {
            i32::try_from(*digits).ok()
        }
        Some(Value::Int(_)) => {
            return Err(invalid_argument(
                "round",
                "the number of fraction digits must be between 0 and 15",
            ));
        }
        Some(other) => return Err(type_error("the digits of `round`", &["int"], other)),
    };
    match (args.get(0), digits) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::Int(int), _) => Ok(Value::Int(*int)),
        (Value::Number(float), None) => float_to_int(float.round()),
        (Value::Number(float), Some(digits)) => {
            let scale = 10_f64.powi(digits);
            Ok(Value::Number((float * scale).round() / scale))
        }
        (other, _) => Err(type_error(&context("round"), &["int", "number"], other)),
    }
}

fn float_to_int(float: f64) -> Result<Value> {
    if float.is_nan() || float < -I64_LIMIT || float >= I64_LIMIT {
        return Err(overflow("`round`", "int"));
    }
    // The value is a whole number in [-2^63, 2^63), so the conversion is exact.
    Ok(Value::Int(float as i64))
}

/// `min` and `max`: over the arguments, or over the items of a single list
/// argument; `null` items are skipped.
fn extreme(function: &str, args: &Args, wanted: Ordering) -> Result<Value> {
    args.at_least(function, 1)?;
    let items: &[Value] = match args.values.as_slice() {
        [Value::List(items)] => items,
        values => values,
    };
    let mut best: Option<&Value> = None;
    for item in items.iter().filter(|item| !item.is_null()) {
        let Some(current) = best else {
            best = Some(item);
            continue;
        };
        if !comparable(current, item) {
            return Err(type_error(&context(function), &[current.type_name()], item));
        }
        if compare(item, current) == wanted {
            best = Some(item);
        }
    }
    Ok(best.cloned().unwrap_or(Value::Null))
}

fn contains(args: &Args) -> Result<Value> {
    args.exactly("contains", 2)?;
    let needle = args.get(1);
    let found = match args.get(0) {
        Value::Null => false,
        Value::List(items) => items.iter().any(|item| equal(item, needle)),
        Value::Map(map) => needle.as_str().is_some_and(|key| map.contains_key(key)),
        haystack => text_of("contains", haystack)?.contains(text_of("contains", needle)?),
    };
    Ok(Value::Bool(found))
}

fn affix(function: &str, args: &Args, test: fn(&str, &str) -> bool) -> Result<Value> {
    args.exactly(function, 2)?;
    match args.get(0) {
        Value::Null => Ok(Value::Bool(false)),
        value => {
            let text = text_of(function, value)?;
            let affix = text_of(function, args.get(1))?;
            Ok(Value::Bool(test(text, affix)))
        }
    }
}

fn replace_text(args: &Args) -> Result<Value> {
    args.exactly("replace_text", 3)?;
    let from = text_of("replace_text", args.get(1))?;
    let to = text_of("replace_text", args.get(2))?;
    if from.is_empty() {
        return Err(invalid_argument(
            "replace_text",
            "the text to replace must not be empty",
        ));
    }
    map_text("replace_text", args.get(0), |text| text.replace(from, to))
}

fn regex(args: &Args, regexes: &RegexCache) -> Result<Value> {
    args.between("regex", 1, 2)?;
    let pattern = text_of("regex", args.get(0))?;
    let flags = match args.values.get(1) {
        None => "",
        Some(flags) => text_of("regex", flags)?,
    };
    let regex = Regex::new(pattern, flags)
        .map_err(|error| Error::new(ErrorCode::Regex, error.to_string()).with_help("regex"))?;
    regexes.compile(&regex)?;
    Ok(Value::Regex(regex))
}

fn time(args: &Args, env: &dyn EvalEnv) -> Result<Value> {
    args.exactly("time", 1)?;
    match args.get(0) {
        Value::Null => Ok(Value::Null),
        Value::Instant(instant) => Ok(Value::Instant(*instant)),
        Value::Text(text) => parse_instant(text, &env.tz())
            .map(Value::Instant)
            .map_err(|error| invalid_value(&error)),
        other => Err(type_error(&context("time"), &["text", "instant"], other)),
    }
}

/// `ago(d)` is `now() - d`; `from_now(d)` is `now() + d`.
fn shifted_now(function: &str, args: &Args, env: &dyn EvalEnv, back: bool) -> Result<Value> {
    args.exactly(function, 1)?;
    match args.typed(0, ValueType::Duration, env)? {
        Value::Null => Ok(Value::Null),
        Value::Duration(duration) => {
            let now = env.now();
            let shifted = if back {
                now.checked_sub(duration)
            } else {
                now.checked_add(duration)
            };
            shifted
                .map(Value::Instant)
                .ok_or_else(|| overflow(&context(function), "instant"))
        }
        other => Err(type_error(&context(function), &["duration"], &other)),
    }
}

fn parent(args: &Args, env: &dyn EvalEnv) -> Result<Value> {
    args.exactly("parent", 1)?;
    match args.typed(0, ValueType::Path, env)? {
        Value::Null => Ok(Value::Null),
        Value::Path(text) => Ok(path::parent(&text).map_or(Value::Null, Value::Path)),
        other => Err(type_error(&context("parent"), &["path"], &other)),
    }
}

fn join_path(args: &Args, env: &dyn EvalEnv) -> Result<Value> {
    args.at_least("join_path", 2)?;
    let mut joined = match args.typed(0, ValueType::Path, env)? {
        Value::Path(text) => text,
        Value::Null => return Ok(Value::Null),
        other => return Err(type_error(&context("join_path"), &["path"], &other)),
    };
    for segment in args.values.iter().skip(1) {
        match segment {
            Value::Text(text) | Value::Path(text) => joined = path::join(&joined, text),
            other => {
                return Err(type_error(
                    "a segment of `join_path`",
                    &["text", "path"],
                    other,
                ));
            }
        }
    }
    Ok(Value::Path(joined))
}

fn base64_encode(args: &Args) -> Result<Value> {
    args.exactly("base64_encode", 1)?;
    match args.get(0) {
        Value::Null => Ok(Value::Null),
        Value::Bytes(bytes) => Ok(Value::Text(bytes.to_base64())),
        Value::Text(text) => Ok(Value::Text(Bytes::from(text.as_bytes()).to_base64())),
        other => Err(type_error(
            &context("base64_encode"),
            &["text", "bytes"],
            other,
        )),
    }
}

fn base64_decode(args: &Args) -> Result<Value> {
    args.exactly("base64_decode", 1)?;
    match args.get(0) {
        Value::Null => Ok(Value::Null),
        Value::Text(text) => Bytes::from_base64(text)
            .map(Value::Bytes)
            .map_err(|error| invalid_value(&error)),
        other => Err(type_error(&context("base64_decode"), &["text"], other)),
    }
}

fn idna_function(
    function: &str,
    args: &Args,
    convert: fn(&str) -> Option<String>,
) -> Result<Value> {
    args.exactly(function, 1)?;
    match args.get(0) {
        Value::Null => Ok(Value::Null),
        Value::Text(text) => convert(text).map(Value::Text).ok_or_else(|| {
            invalid_argument(function, &format!("`{text}` is not a valid domain name"))
        }),
        other => Err(type_error(&context(function), &["text"], other)),
    }
}

fn hex(args: &Args) -> Result<Value> {
    args.exactly("hex", 1)?;
    let text = match args.get(0) {
        Value::Null => return Ok(Value::Null),
        Value::Bytes(bytes) => hex_digits(bytes.as_slice()),
        Value::Text(text) => hex_digits(text.as_bytes()),
        Value::Int(int) if *int < 0 => format!("-{:x}", int.unsigned_abs()),
        Value::Int(int) => format!("{int:x}"),
        other => {
            return Err(type_error(
                &context("hex"),
                &["bytes", "text", "int"],
                other,
            ));
        }
    };
    Ok(Value::Text(text))
}

fn hex_digits(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn uuid(args: &Args, env: &dyn EvalEnv) -> Result<Value> {
    args.exactly("uuid", 0)?;
    let mut bytes = [0_u8; UUID_BYTES];
    let (high, low) = bytes.split_at_mut(UUID_BYTES / 2);
    high.copy_from_slice(&env.random_u64().to_be_bytes());
    low.copy_from_slice(&env.random_u64().to_be_bytes());
    let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
    Ok(Value::Text(id.hyphenated().to_string()))
}

fn random(args: &Args, env: &dyn EvalEnv) -> Result<Value> {
    args.between("random", 0, 1)?;
    match args.values.first() {
        None => {
            let bits = env.random_u64() >> (BITS_PER_U64 - RANDOM_BITS);
            Ok(Value::Number(bits as f64 * RANDOM_SCALE))
        }
        Some(Value::Range(range)) => random_in(*range, env),
        Some(other) => Err(type_error(&context("random"), &["range"], other)),
    }
}

/// Uniform integer of a closed range, by scaling 64 random bits.
fn random_in(range: Range, env: &dyn EvalEnv) -> Result<Value> {
    let (Some(start), Some(end)) = (range.start(), range.end()) else {
        return Err(invalid_argument(
            "random",
            "the range needs both bounds, for example 1..6",
        ));
    };
    let span = u128::try_from(i128::from(end) - i128::from(start) + 1).unwrap_or(1);
    let offset = (u128::from(env.random_u64()) * span) >> BITS_PER_U64;
    let value = i128::from(start) + i128::try_from(offset).unwrap_or(0);
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| overflow("`random`", "int"))
}
