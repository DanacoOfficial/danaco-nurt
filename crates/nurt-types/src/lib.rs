// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Values, records, units and ranges of the Danaco Nurt language.
//!
//! The crate gives the value model of `nurt_contracts::value` its behaviour:
//! text forms of sizes, durations, instants and ranges, the total order of
//! values, the implicit conversions of the language, arithmetic, record field
//! access and the evaluator of bound expressions with the built-in functions.
//! It performs no input or output and contains no platform-specific code; the
//! clock, the random source, the local time rules and the named results reach the
//! evaluator through [`EvalEnv`].
//!
//! | Area | Items |
//! |---|---|
//! | text forms | [`parse_size`], [`format_size`], [`display_size`], [`parse_duration`], [`format_duration`], [`display_duration`], [`parse_instant`], [`display_instant`], [`parse_range`], [`display_short`] |
//! | order | [`compare`], [`equal`], [`comparable`], [`OrderedValue`] |
//! | conversions | [`ValueType`], [`converts_implicitly`], [`coerce`], [`coerce_in`] |
//! | arithmetic | [`add`], [`sub`], [`mul`], [`div`], [`neg`] |
//! | fields | [`get_field`], [`get_path`], [`field_names`] |
//! | evaluation | [`Evaluator`], [`EvalEnv`], [`ClockEnv`], [`FixedEnv`], [`RandomSource`], [`FUNCTIONS`] |
//!
//! # Expressions
//!
//! Operators, from the highest precedence: `.` (field), call, unary `-` and
//! `not`, `* /`, `+ -`, `== != < <= > >= ~ !~ in`, `and`, `or`; the bound
//! tree already encodes the precedence.
//!
//! * `and` and `or` evaluate the right operand only when the left one does
//!   not decide the result; `null` counts as `false` in `and`, `or` and
//!   `not`.
//! * `==` and `!=` compare any two values with [`equal`]; `<`, `<=`, `>`
//!   and `>=` need operands of one type family and give `false` when an
//!   operand is `null`.
//! * In a comparison, a text literal facing a `path`, `glob`, `instant`,
//!   `key`, `size` or `duration` converts to that type
//!   (`modified > "2026-11-01"`); other values never convert. The binder
//!   raises the errors of such a comparison before the evaluation, for a
//!   field of a known type, with [`operation_error`] and [`function_error`].
//! * `~` and `!~` match text-like values against a `regex`.
//! * `in` tests membership in a list, an integer range or the keys of a
//!   map.
//! * Arithmetic follows [`add`], [`sub`], [`mul`] and [`div`].
//!
//! # Examples
//!
//! ```rust
//! use nurt_contracts::ast::BinOp;
//! use nurt_contracts::bound::BoundExpr;
//! use nurt_contracts::value::{Instant, Record, RecordKind, Size, Value};
//! use nurt_types::{Evaluator, FixedEnv};
//!
//! let record = Record::new(RecordKind::Entry).with("size", Value::Size(Size::from_bytes(2_000_000)));
//! let expr = BoundExpr::Bin {
//!     op: BinOp::Gt,
//!     l: Box::new(BoundExpr::Field { name: "size".to_owned(), of: None }),
//!     r: Box::new(BoundExpr::Size { v: 1_000_000 }),
//! };
//! let env = FixedEnv::new(Instant::UNIX_EPOCH, 1);
//! assert_eq!(Evaluator::new().eval(&expr, Some(&record), &env)?, Value::Bool(true));
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod access;
mod arith;
mod builtins;
mod check;
mod coerce;
mod compare;
mod display;
mod env;
mod errors;
mod eval;
mod idna;
mod path;
mod pattern;
mod units;

pub use access::{field_names, get_field, get_path};
pub use arith::{add, div, mul, neg, sub};
pub use builtins::FUNCTIONS;
pub use check::{Operand, function_error, operation_error};
pub use coerce::{ValueType, coerce, coerce_in, converts_implicitly};
pub use compare::{OrderedValue, comparable, compare, equal};
pub use display::display_short;
pub use env::{ClockEnv, EvalEnv, FixedEnv, RandomSource, instant_from_wall};
pub use eval::Evaluator;
pub use units::{
    display_duration, display_instant, display_size, format_duration, format_size, parse_duration,
    parse_instant, parse_range, parse_size,
};
