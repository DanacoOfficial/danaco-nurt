// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-types::env
// Responsibility:   Environment of the evaluator: time, randomness, local time rules and.
// Dependencies:     jiff, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Environment of the evaluator: time, randomness, local time rules and
//! named results.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use jiff::tz::TimeZone;
use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::value::{Fields, Instant, RecordKind, Value};

/// What the evaluator reads from outside the expression.
///
/// The evaluator never reads the system clock or a global random source,
/// so an evaluation is reproducible for a given environment. The executor
/// builds the environment from the clock of the execution context
/// ([`ClockEnv`]); tests use [`FixedEnv`].
pub trait EvalEnv: Send + Sync {
    /// Current time: the value of `now()` and the base of `ago` and
    /// `from_now`.
    fn now(&self) -> Instant;

    /// Next 64 random bits: the source of `random()` and `uuid()`.
    fn random_u64(&self) -> u64;

    /// Local time rules for instants written without an offset; UTC when
    /// not overridden.
    fn tz(&self) -> TimeZone {
        TimeZone::UTC
    }

    /// Value of the named result `@name`: a list of records, or the value
    /// of a `let` that produced a single value.
    fn named(&self, _name: &str) -> Option<&Value> {
        None
    }

    /// Whether `name` is a field of records of `kind` that may be absent
    /// from a record, such as a lazy field; reading it gives `null` instead
    /// of `E_UNKNOWN_FIELD`.
    fn is_known_field(&self, _kind: RecordKind, _name: &str) -> bool {
        false
    }
}

/// Seeded source of random numbers, safe to share between threads.
///
/// # Examples
///
/// ```rust
/// use nurt_types::RandomSource;
///
/// let first = RandomSource::with_seed(7);
/// let second = RandomSource::with_seed(7);
/// assert_eq!(first.next_u64(), second.next_u64());
/// ```
#[derive(Debug)]
pub struct RandomSource(Mutex<fastrand::Rng>);

impl RandomSource {
    /// Source seeded from the entropy of the process.
    #[must_use]
    pub fn new() -> Self {
        Self(Mutex::new(fastrand::Rng::new()))
    }

    /// Source with a fixed seed: the same seed gives the same sequence.
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self(Mutex::new(fastrand::Rng::with_seed(seed)))
    }

    /// Next 64 random bits.
    #[must_use]
    pub fn next_u64(&self) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .u64(..)
    }
}

impl Default for RandomSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Instant of a wall-clock reading, limited to the range of [`Instant`].
#[must_use]
pub fn instant_from_wall(time: WallTime) -> Instant {
    let nanos = time
        .unix_nanos()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    Instant::from_unix_nanos(i64::try_from(nanos).unwrap_or_default())
}

/// Environment over a [`Clock`] and a shared [`RandomSource`], as built by
/// the executor for each evaluation.
pub struct ClockEnv<'a> {
    clock: &'a dyn Clock,
    random: &'a RandomSource,
    tz: TimeZone,
    names: Option<&'a Fields>,
}

impl<'a> ClockEnv<'a> {
    /// Environment reading time from `clock` and random numbers from
    /// `random`, with UTC and no named results.
    #[must_use]
    pub fn new(clock: &'a dyn Clock, random: &'a RandomSource) -> Self {
        Self {
            clock,
            random,
            tz: TimeZone::UTC,
            names: None,
        }
    }

    /// Sets the local time rules.
    #[must_use]
    pub fn with_tz(mut self, tz: TimeZone) -> Self {
        self.tz = tz;
        self
    }

    /// Sets the named results, by name without `@`.
    #[must_use]
    pub fn with_names(mut self, names: &'a Fields) -> Self {
        self.names = Some(names);
        self
    }
}

impl fmt::Debug for ClockEnv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClockEnv")
            .field("tz", &self.tz)
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

impl EvalEnv for ClockEnv<'_> {
    fn now(&self) -> Instant {
        instant_from_wall(self.clock.now())
    }

    fn random_u64(&self) -> u64 {
        self.random.next_u64()
    }

    fn tz(&self) -> TimeZone {
        self.tz.clone()
    }

    fn named(&self, name: &str) -> Option<&Value> {
        self.names.and_then(|names| names.get(name))
    }
}

/// Environment with a fixed time, a seeded random source and explicit named
/// results, for tests and for evaluations that must be reproducible.
///
/// # Examples
///
/// ```rust
/// use nurt_contracts::value::{Instant, Value};
/// use nurt_types::{EvalEnv, FixedEnv};
///
/// let env = FixedEnv::new(Instant::UNIX_EPOCH, 42).with_name("limit", Value::Int(10));
/// assert_eq!(env.now(), Instant::UNIX_EPOCH);
/// assert_eq!(env.named("limit"), Some(&Value::Int(10)));
/// ```
#[derive(Debug)]
pub struct FixedEnv {
    now: Instant,
    random: RandomSource,
    tz: TimeZone,
    names: Fields,
    known_fields: Vec<(RecordKind, String)>,
}

impl FixedEnv {
    /// Environment at the instant `now` with random numbers from `seed`,
    /// in UTC.
    #[must_use]
    pub fn new(now: Instant, seed: u64) -> Self {
        Self {
            now,
            random: RandomSource::with_seed(seed),
            tz: TimeZone::UTC,
            names: Fields::new(),
            known_fields: Vec::new(),
        }
    }

    /// Sets the local time rules.
    #[must_use]
    pub fn with_tz(mut self, tz: TimeZone) -> Self {
        self.tz = tz;
        self
    }

    /// Adds the named result `@name`.
    #[must_use]
    pub fn with_name(mut self, name: &str, value: Value) -> Self {
        self.names.insert(name.to_owned(), value);
        self
    }

    /// Declares `name` a field of `kind` that may be absent.
    #[must_use]
    pub fn with_known_field(mut self, kind: RecordKind, name: &str) -> Self {
        self.known_fields.push((kind, name.to_owned()));
        self
    }
}

impl EvalEnv for FixedEnv {
    fn now(&self) -> Instant {
        self.now
    }

    fn random_u64(&self) -> u64 {
        self.random.next_u64()
    }

    fn tz(&self) -> TimeZone {
        self.tz.clone()
    }

    fn named(&self, name: &str) -> Option<&Value> {
        self.names.get(name)
    }

    fn is_known_field(&self, kind: RecordKind, name: &str) -> bool {
        self.known_fields
            .iter()
            .any(|(known_kind, known_name)| *known_kind == kind && known_name == name)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
