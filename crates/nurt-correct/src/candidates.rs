// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::candidates
// Responsibility:   Candidates of an ambiguous program and their acceptance in one step.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Candidates of an ambiguous program and their acceptance in one step.
//!
//! A program that cannot be corrected without guessing returns
//! `E_AMBIGUOUS` with at most [`MAX_CANDIDATES`] full programs in canonical
//! form, ranked by [`rank`]. Each set of candidates is kept by a [`Store`] of
//! the running instance, so that the caller can send `{"accept": "c1"}`
//! instead of the program text. The store keeps the [`STORE_CAPACITY`] most
//! recently used sets for the configured lifetime; an expired or unknown
//! identifier gives `E_EXPIRED_CANDIDATE`.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::atomic::{self, AtomicU64};
use std::time::{Duration, Instant};

use nurt_contracts::config::CorrectionConfig;
use nurt_contracts::error::{self, Error, ErrorCode, Stage};

/// Largest number of candidates of one ambiguous program.
pub const MAX_CANDIDATES: usize = 3;

/// Number of candidate sets kept by a [`Store`].
pub const STORE_CAPACITY: usize = 8;

/// Default lifetime of a candidate set (`correction.candidates_ttl`).
pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);

/// Prefix of candidate identifiers (`c1`, `c2`, …).
pub const ID_PREFIX: char = 'c';

/// A corrected program offered as a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Full program in canonical form.
    pub program: String,
    /// One sentence in English that explains how the candidate differs.
    pub why: String,
    /// Number of probable (class P) corrections in the candidate.
    pub probable: usize,
    /// Size of the target set as measured by the caller (narrower pattern,
    /// smaller unit); smaller ranks first.
    pub targets: u64,
    /// Edit distance between the candidate and the program as written.
    pub distance: usize,
}

impl Candidate {
    fn order(&self, other: &Self) -> Ordering {
        (self.probable, self.targets, self.distance, &self.program).cmp(&(
            other.probable,
            other.targets,
            other.distance,
            &other.program,
        ))
    }
}

/// Ranks candidates and keeps at most [`MAX_CANDIDATES`].
///
/// Order: fewer probable corrections, then a smaller target set, then a
/// smaller edit distance to the input, then the program text. Candidates with
/// the same program text are listed once.
///
/// # Examples
///
/// ```
/// use nurt_correct::candidates::{Candidate, rank};
///
/// let candidate = |program: &str, probable, targets| Candidate {
///     program: program.into(),
///     why: String::new(),
///     probable,
///     targets,
///     distance: 1,
/// };
/// let ranked = rank(vec![candidate("b", 1, 1), candidate("a", 0, 5), candidate("c", 0, 2)]);
/// let order: Vec<&str> = ranked.iter().map(|c| c.program.as_str()).collect();
/// assert_eq!(order, ["c", "a", "b"]);
/// ```
#[must_use]
pub fn rank(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by(Candidate::order);
    let mut seen = Vec::with_capacity(MAX_CANDIDATES);
    candidates.retain(|candidate| {
        if seen.contains(&candidate.program) {
            false
        } else {
            seen.push(candidate.program.clone());
            true
        }
    });
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

/// Source of monotonic time for candidate lifetimes.
pub trait Clock {
    /// Time elapsed since a fixed origin of this clock.
    fn now(&self) -> Duration;
}

/// Monotonic clock of the running process.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// A clock whose origin is the moment of creation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Clock advanced explicitly, for deterministic tests and replays.
#[derive(Debug, Default)]
pub struct ManualClock {
    millis: AtomicU64,
}

impl ManualClock {
    /// A clock at its origin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: Duration) {
        let millis = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.millis.fetch_add(millis, atomic::Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.load(atomic::Ordering::Relaxed))
    }
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> Duration {
        (**self).now()
    }
}

#[derive(Debug, Clone)]
struct Set {
    created: Duration,
    entries: Vec<(String, String)>,
}

/// Candidate sets of the running instance.
///
/// The store keeps [`STORE_CAPACITY`] sets; adding a set to a full store
/// drops the least recently used one. Identifiers grow within one store and
/// are never reused, so an identifier of a dropped set stays invalid.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_contracts::error::ErrorCode;
/// use nurt_correct::candidates::{Candidate, ManualClock, Store};
///
/// let clock = ManualClock::new();
/// let mut store = Store::new(Duration::from_secs(600), &clock);
/// let offered = store.insert(vec![Candidate {
///     program: "remove \"/srv/a\"".into(),
///     why: "Corrects the command name.".into(),
///     probable: 0,
///     targets: 1,
///     distance: 1,
/// }]);
/// assert_eq!(offered[0].reference.as_deref(), Some("c1"));
/// assert_eq!(store.accept("c1").unwrap(), "remove \"/srv/a\"");
/// clock.advance(Duration::from_secs(601));
/// assert_eq!(store.accept("c1").unwrap_err().code(), ErrorCode::ExpiredCandidate);
/// ```
#[derive(Debug)]
pub struct Store<C: Clock = SystemClock> {
    ttl: Duration,
    clock: C,
    next: u64,
    sets: VecDeque<Set>,
}

impl Store<SystemClock> {
    /// A store with the lifetime of `correction.candidates_ttl` and the
    /// process clock.
    #[must_use]
    pub fn from_config(config: &CorrectionConfig) -> Self {
        Self::new(config.candidates_ttl.as_std(), SystemClock::new())
    }
}

impl<C: Clock> Store<C> {
    /// An empty store with the given lifetime and clock.
    #[must_use]
    pub fn new(ttl: Duration, clock: C) -> Self {
        Self {
            ttl,
            clock,
            next: 0,
            sets: VecDeque::with_capacity(STORE_CAPACITY),
        }
    }

    /// Ranks `candidates`, keeps them as one set and returns them as error
    /// candidates with identifiers.
    pub fn insert(&mut self, candidates: Vec<Candidate>) -> Vec<error::Candidate> {
        self.expire();
        let ranked = rank(candidates);
        let mut offered = Vec::with_capacity(ranked.len());
        let mut entries = Vec::with_capacity(ranked.len());
        for (position, candidate) in ranked.into_iter().enumerate() {
            self.next += 1;
            let id = format!("{ID_PREFIX}{}", self.next);
            offered.push(error::Candidate {
                rank: u32::try_from(position + 1).unwrap_or(u32::MAX),
                reference: Some(id.clone()),
                program: candidate.program.clone(),
                why: Some(candidate.why),
            });
            entries.push((id, candidate.program));
        }
        if self.sets.len() == STORE_CAPACITY {
            self.sets.pop_front();
        }
        self.sets.push_back(Set {
            created: self.clock.now(),
            entries,
        });
        offered
    }

    /// Returns the program of the candidate `id` and marks its set as most
    /// recently used.
    ///
    /// # Errors
    ///
    /// Returns `E_EXPIRED_CANDIDATE` when the identifier is unknown, its set
    /// was dropped, or its lifetime is over.
    pub fn accept(&mut self, id: &str) -> Result<String, Error> {
        self.expire();
        let position = self
            .sets
            .iter()
            .position(|set| set.entries.iter().any(|(entry, _)| entry == id))
            .ok_or_else(|| expired(id))?;
        let set = self.sets.remove(position).ok_or_else(|| expired(id))?;
        let program = set
            .entries
            .iter()
            .find(|(entry, _)| entry == id)
            .map(|(_, program)| program.clone());
        self.sets.push_back(set);
        program.ok_or_else(|| expired(id))
    }

    /// Number of sets kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    /// Whether no set is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    fn expire(&mut self) {
        let now = self.clock.now();
        let ttl = self.ttl;
        self.sets
            .retain(|set| now.saturating_sub(set.created) <= ttl);
    }
}

fn expired(id: &str) -> Error {
    Error::new(
        ErrorCode::ExpiredCandidate,
        format!(
            "candidate `{id}` is unknown or expired; send the program text of the candidate instead"
        ),
    )
    .with_stage(Stage::Parse)
    .with_found(id)
    .with_help("corrections")
}

/// Builds `E_AMBIGUOUS` with the offered candidates.
#[must_use]
pub fn ambiguous(message: impl Into<String>, offered: Vec<error::Candidate>) -> Error {
    offered.into_iter().fold(
        Error::new(ErrorCode::Ambiguous, message).with_help("corrections"),
        Error::with_candidate,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(program: &str) -> Candidate {
        Candidate {
            program: program.into(),
            why: format!("Reads the program as {program}."),
            probable: 0,
            targets: 0,
            distance: 0,
        }
    }

    #[test]
    fn ranks_by_probable_corrections_targets_and_distance() {
        let mut a = candidate("a");
        a.distance = 2;
        let mut b = candidate("b");
        b.distance = 1;
        let mut c = candidate("c");
        c.targets = 1;
        let mut d = candidate("d");
        d.probable = 1;
        let ranked = rank(vec![d, c, a.clone(), b, a]);
        assert_eq!(
            ranked
                .iter()
                .map(|c| c.program.as_str())
                .collect::<Vec<_>>(),
            ["b", "a", "c"]
        );
    }

    #[test]
    fn keeps_at_most_three_ranked_candidates() {
        let ranked = rank(["d", "c", "b", "a"].map(candidate).to_vec());
        assert_eq!(
            ranked
                .iter()
                .map(|c| c.program.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(rank(Vec::new()).is_empty());
    }

    #[test]
    fn an_empty_set_is_kept_but_offers_nothing() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        assert!(store.insert(Vec::new()).is_empty());
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.accept("c1").unwrap_err().code(),
            ErrorCode::ExpiredCandidate
        );
    }

    #[test]
    fn keeps_multibyte_programs_exactly() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        let offered = store.insert(vec![candidate("remove \"/srv/zażółć\"")]);
        assert_eq!(offered[0].program, "remove \"/srv/zażółć\"");
        assert_eq!(store.accept("c1").unwrap(), "remove \"/srv/zażółć\"");
    }

    #[test]
    fn numbers_candidates_without_reuse() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        assert!(store.is_empty());
        let first = store.insert(vec![candidate("x"), candidate("y")]);
        let second = store.insert(vec![candidate("z")]);
        let ids: Vec<_> = first
            .iter()
            .chain(&second)
            .filter_map(|c| c.reference.clone())
            .collect();
        assert_eq!(ids, ["c1", "c2", "c3"]);
        assert_eq!(first[1].rank, 2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.accept("c2").unwrap(), "y");
    }

    #[test]
    fn keeps_the_eight_most_recently_used_sets() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        for index in 0..STORE_CAPACITY {
            store.insert(vec![candidate(&format!("p{index}"))]);
        }
        assert_eq!(store.accept("c1").unwrap(), "p0");
        store.insert(vec![candidate("p8")]);
        assert_eq!(store.len(), STORE_CAPACITY);
        assert_eq!(store.accept("c1").unwrap(), "p0");
        assert_eq!(
            store.accept("c2").unwrap_err().code(),
            ErrorCode::ExpiredCandidate
        );
        assert_eq!(store.accept("c9").unwrap(), "p8");
    }

    #[test]
    fn expires_sets_after_the_lifetime() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        store.insert(vec![candidate("x")]);
        clock.advance(DEFAULT_TTL);
        assert_eq!(store.accept("c1").unwrap(), "x");
        clock.advance(Duration::from_millis(1));
        let error = store.accept("c1").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ExpiredCandidate);
        assert_eq!(error.stage(), Some(Stage::Parse));
        assert_eq!(error.found.as_deref(), Some("c1"));
        assert!(store.is_empty());
    }

    #[test]
    fn rejects_unknown_identifiers() {
        let mut store = Store::from_config(&CorrectionConfig::default());
        assert_eq!(
            store.accept("c1").unwrap_err().code(),
            ErrorCode::ExpiredCandidate
        );
        assert_eq!(
            store.accept("").unwrap_err().code(),
            ErrorCode::ExpiredCandidate
        );
    }

    #[test]
    fn builds_ambiguous_errors() {
        let clock = ManualClock::new();
        let mut store = Store::new(DEFAULT_TTL, &clock);
        let offered = store.insert(vec![candidate("a"), candidate("b")]);
        let error = ambiguous("the size unit `10M` has two readings", offered);
        assert_eq!(error.code(), ErrorCode::Ambiguous);
        assert_eq!(error.candidates.len(), 2);
        assert_eq!(error.candidates[0].program, "a");
        assert!(SystemClock::default().now() < Duration::from_secs(1));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
