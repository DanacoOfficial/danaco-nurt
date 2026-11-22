// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language types and the implicit conversions between them.
//!
//! The only implicit conversions of the language turn a text written in a
//! place of type `path`, `glob`, `instant`, `key`, `size` or `duration` into
//! a value of that type. A text never becomes a `regex` implicitly: a
//! regular expression is written `/pattern/` or `regex("pattern")`. Named
//! results (`@name`) and computed expressions are never converted.

use std::fmt;
use std::str::FromStr;

use jiff::tz::TimeZone;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Value, ValueError};
use serde_json::json;

use crate::errors::{invalid_value, type_error};
use crate::units::{parse_duration, parse_instant, parse_size};

macro_rules! value_types {
    ($( $(#[$meta:meta])* $variant:ident = $name:literal, )+) => {
        /// Type of a language value, as named in signatures and errors.
        ///
        /// `int` covers integers kept as source text and `number` covers
        /// decimals kept as source text.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum ValueType {
            $( $(#[$meta])* $variant, )+
        }

        impl ValueType {
            /// Every type in a stable order.
            pub const ALL: &'static [ValueType] = &[$(ValueType::$variant),+];

            /// Name of the type, the same as [`Value::type_name`].
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(ValueType::$variant => $name,)+
                }
            }
        }

        impl FromStr for ValueType {
            type Err = ValueError;

            fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
                match text {
                    $($name => Ok(ValueType::$variant),)+
                    _ => Err(ValueError::new("type", text, "unknown type name")),
                }
            }
        }
    };
}

value_types! {
    /// `null`.
    Null = "null",
    /// `bool`.
    Bool = "bool",
    /// `int`.
    Int = "int",
    /// `number`.
    Number = "number",
    /// `text`.
    Text = "text",
    /// `size`.
    Size = "size",
    /// `duration`.
    Duration = "duration",
    /// `instant`.
    Instant = "instant",
    /// `date`, a date or time kept in its document form.
    Date = "date",
    /// `path`.
    Path = "path",
    /// `glob`.
    Glob = "glob",
    /// `regex`.
    Regex = "regex",
    /// `key`.
    Key = "key",
    /// `bytes`.
    Bytes = "bytes",
    /// `range`.
    Range = "range",
    /// `enum`, a value of an enumeration.
    Enum = "enum",
    /// `list`.
    List = "list",
    /// `map`.
    Map = "map",
    /// `record`.
    Record = "record",
}

impl ValueType {
    /// Type of the value; `None` for a kind of value this version does not
    /// know.
    #[must_use]
    pub fn of(value: &Value) -> Option<Self> {
        value.type_name().parse().ok()
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a value of type `from` written as a literal converts implicitly
/// to `to`: only `text` to `path`, `glob`, `instant`, `key`, `size` and
/// `duration`.
///
/// # Examples
///
/// ```rust
/// use nurt_types::{ValueType, converts_implicitly};
///
/// assert!(converts_implicitly(ValueType::Text, ValueType::Size));
/// assert!(!converts_implicitly(ValueType::Text, ValueType::Regex));
/// assert!(!converts_implicitly(ValueType::Int, ValueType::Number));
/// ```
#[must_use]
pub const fn converts_implicitly(from: ValueType, to: ValueType) -> bool {
    matches!(
        (from, to),
        (
            ValueType::Text,
            ValueType::Path
                | ValueType::Glob
                | ValueType::Instant
                | ValueType::Key
                | ValueType::Size
                | ValueType::Duration
        )
    )
}

/// Converts a literal value to the type of its place, reading texts
/// without an offset as UTC; see [`coerce_in`].
///
/// # Errors
///
/// As [`coerce_in`].
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::{Size, Value};
/// use nurt_types::{ValueType, coerce};
///
/// let size = coerce(Value::from("10mb"), ValueType::Size)?;
/// assert_eq!(size, Value::Size(Size::from_bytes(10_000_000)));
/// assert!(coerce(Value::from("a+"), ValueType::Regex).is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn coerce(value: Value, target: ValueType) -> Result<Value> {
    coerce_in(value, target, &TimeZone::UTC)
}

/// Converts a literal value to the type of its place.
///
/// A value that already has the target type and `null` are returned
/// unchanged. A text converts to the types listed in
/// [`converts_implicitly`]; instants without an offset are read in the
/// local time rules `tz`. Paths, globs and keys keep their text; checking
/// their form belongs to binding.
///
/// # Errors
///
/// * `E_INVALID_VALUE` when the text does not form a value of the target
///   type, for example `10q` for a size;
/// * `E_TYPE` with the target type in `expected` for every other pair of
///   types, including text to `regex`.
pub fn coerce_in(value: Value, target: ValueType, tz: &TimeZone) -> Result<Value> {
    if value.is_null() || ValueType::of(&value) == Some(target) {
        return Ok(value);
    }
    match (value, target) {
        (Value::Text(text), ValueType::Path) => Ok(Value::Path(text)),
        (Value::Text(text), ValueType::Glob) => Ok(Value::Glob(text)),
        (Value::Text(text), ValueType::Key) => Ok(Value::Key(text)),
        (Value::Text(text), ValueType::Instant) => parse_instant(&text, tz)
            .map(Value::Instant)
            .map_err(|error| invalid_value(&error)),
        (Value::Text(text), ValueType::Size) => parse_size(&text)
            .map(Value::Size)
            .map_err(|error| invalid_value(&error)),
        (Value::Text(text), ValueType::Duration) => parse_duration(&text)
            .map(Value::Duration)
            .map_err(|error| invalid_value(&error)),
        (Value::Text(_), ValueType::Regex) => Err(text_is_not_regex()),
        (value, target) => Err(type_error("the place", &[target.as_str()], &value)),
    }
}

/// `E_TYPE` for a text where a regular expression is required.
pub(crate) fn text_is_not_regex() -> Error {
    Error::new(
        ErrorCode::Type,
        "a text is not a regular expression; write /pattern/ or regex(\"pattern\")",
    )
    .with_expected(json!(["regex"]))
    .with_actual(json!("text"))
    .with_help("regex")
}
