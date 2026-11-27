// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::retention::age
// Responsibility:   Entry age evaluation.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Entry age evaluation.
//!
//! The age of an entry is measured by the clock selected by
//! `journal.age_clock`:
//!
//! * `wall`: real elapsed time, including suspend, hibernation and
//!   power-off. The effective wall time never moves backwards: it is the
//!   larger of the current wall time and the effective wall time of the last
//!   observation advanced, within the same boot, by the suspend-inclusive
//!   monotonic clock. Within the boot of the commit the age is also at least
//!   the suspend-inclusive monotonic delta. A wall clock moved backwards
//!   therefore never extends the life of an entry; a wall clock moved
//!   forwards shortens it.
//! * `active`: running time only. The persisted [`AgeState`] accumulates the
//!   running time of every boot; suspend and power-off do not count.
//!
//! The age of an entry never decreases between evaluations as long as every
//! evaluation is followed by [`AgeState::observe`] and the state is persisted
//! in the state directory as [`AGE_STATE_FILE`].

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::AgeClock;
use nurt_contracts::env::{BootId, Clock, ClockStamp, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FsWrite};
use nurt_contracts::stage::{CancelReason, CancelToken};
use serde::{Deserialize, Serialize};

use crate::manifest;

/// Name of the file that keeps [`AgeState`] in the state directory of the
/// account.
pub(crate) const AGE_STATE_FILE: &str = "age-state.json";

/// Persisted clock observations needed to evaluate entry age across clock
/// changes, suspend and reboots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgeState {
    /// Effective wall time of the last observation.
    pub(crate) last_seen_wall: Option<WallTime>,
    /// Boot of the last observation.
    pub(crate) last_boot: Option<BootId>,
    /// Suspend-inclusive monotonic reading of the last observation.
    pub(crate) last_suspend_reading: Duration,
    /// Running-time monotonic reading of the last observation.
    pub(crate) last_active_reading: Duration,
    /// Running time accumulated over all boots up to the last observation.
    pub(crate) active_accumulated: Duration,
}

/// Clock readings recorded in an entry when it is committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EntryTimeBasis {
    /// Readings at commit.
    pub(crate) committed: ClockStamp,
    /// Effective wall time at commit (see [`AgeState::effective_wall`]).
    pub(crate) effective_wall: WallTime,
    /// Accumulated running time at commit (see [`AgeState::active_total`]).
    pub(crate) active_at_commit: Duration,
}

impl AgeState {
    /// Effective wall time at `now`.
    pub(crate) fn effective_wall(&self, now: &ClockStamp) -> WallTime {
        let Some(last) = self.last_seen_wall else {
            return now.wall;
        };
        let advanced = if self.same_boot(now) {
            last.saturating_add(
                now.since_boot_with_suspend
                    .saturating_sub(self.last_suspend_reading),
            )
        } else {
            last
        };
        now.wall.max(advanced)
    }

    /// Running time accumulated over all boots up to `now`.
    ///
    /// After a reboot the running time of the previous boot after its last
    /// observation is unknown and not counted, so entries measured by
    /// running time may live slightly longer, never shorter.
    pub(crate) fn active_total(&self, now: &ClockStamp) -> Duration {
        let current_boot = if self.same_boot(now) {
            now.since_boot_active
                .saturating_sub(self.last_active_reading)
        } else {
            now.since_boot_active
        };
        self.active_accumulated.saturating_add(current_boot)
    }

    /// Records the readings of `now`; a new boot closes the accumulation of
    /// the previous one.
    pub(crate) fn observe(&mut self, now: &ClockStamp) {
        let wall = self.effective_wall(now);
        let active = self.active_total(now);
        self.last_seen_wall = Some(wall);
        self.active_accumulated = active;
        self.last_boot = Some(now.boot.clone());
        self.last_suspend_reading = now.since_boot_with_suspend;
        self.last_active_reading = now.since_boot_active;
    }

    /// Content of [`AGE_STATE_FILE`].
    ///
    /// # Errors
    ///
    /// Returns `E_INTERNAL` when the state cannot be serialized, which does
    /// not happen for valid readings.
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|error| {
            Error::new(
                ErrorCode::Internal,
                format!("cannot serialize the journal age state: {error}"),
            )
        })
    }

    /// State read from the content of [`AGE_STATE_FILE`].
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the content is not a valid age state; the caller
    /// starts from an empty state.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("journal age state is not valid and is reset: {error}"),
            )
        })
    }

    fn same_boot(&self, now: &ClockStamp) -> bool {
        self.last_boot.as_ref() == Some(&now.boot)
    }
}

/// Evaluator of entry ages at one moment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AgeEvaluator<'a> {
    state: &'a AgeState,
    now: &'a ClockStamp,
}

impl<'a> AgeEvaluator<'a> {
    /// Evaluator at `now` with the persisted `state`.
    pub(crate) fn new(state: &'a AgeState, now: &'a ClockStamp) -> Self {
        Self { state, now }
    }

    /// Age of an entry committed with `basis`, measured by `clock`.
    pub(crate) fn age(&self, clock: AgeClock, basis: &EntryTimeBasis) -> Duration {
        match clock {
            AgeClock::Wall => {
                let wall = self
                    .state
                    .effective_wall(self.now)
                    .saturating_since(basis.effective_wall);
                let monotonic = if basis.committed.boot == self.now.boot {
                    self.now
                        .since_boot_with_suspend
                        .saturating_sub(basis.committed.since_boot_with_suspend)
                } else {
                    Duration::ZERO
                };
                wall.max(monotonic)
            }
            AgeClock::Active => self
                .state
                .active_total(self.now)
                .saturating_sub(basis.active_at_commit),
        }
    }
}

/// Clock whose stamps carry the effective wall time of the persisted
/// [`AgeState`].
///
/// An entry records the stamp of its commit as the basis of its age. With the
/// raw wall time, an entry committed after the wall clock was moved backwards
/// would start from a basis earlier than the effective wall time of the
/// journal and appear older than it is. Stamps of this clock never fall below
/// the effective wall time, so the basis of every entry is monotonic.
pub(crate) struct EffectiveClock {
    inner: Arc<dyn Clock>,
    fs: Arc<dyn FsWrite>,
    state_file: Option<AbsPath>,
}

impl EffectiveClock {
    /// Clock over `inner` that reads the age state of `state_dir`.
    pub(crate) fn new(inner: Arc<dyn Clock>, fs: Arc<dyn FsWrite>, state_dir: &AbsPath) -> Self {
        Self {
            inner,
            fs,
            state_file: state_dir.join_name(AGE_STATE_FILE).ok(),
        }
    }

    fn state(&self) -> Option<AgeState> {
        let file = self.state_file.as_ref()?;
        let content = manifest::read_bytes(self.fs.as_ref(), file).ok()?;
        AgeState::decode(&content).ok()
    }
}

impl Clock for EffectiveClock {
    fn now(&self) -> WallTime {
        self.inner.now()
    }

    fn monotonic(&self) -> Duration {
        self.inner.monotonic()
    }

    fn stamp(&self) -> ClockStamp {
        let mut stamp = self.inner.stamp();
        if let Some(state) = self.state() {
            stamp.wall = state.effective_wall(&stamp);
        }
        stamp
    }

    fn sleep(
        &self,
        duration: Duration,
        cancel: &CancelToken,
    ) -> std::result::Result<(), CancelReason> {
        self.inner.sleep(duration, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60;
    const HOUR: u64 = 3600;
    /// Wall time of the first commit in the tests: 2026-01-01T00:00:00Z.
    const START: u64 = 1_767_225_600;

    /// Virtual clock: wall time, boot, suspend-inclusive and running-time
    /// readings, all in seconds.
    #[derive(Clone)]
    struct Virtual {
        wall: i128,
        boot: &'static str,
        with_suspend: u64,
        active: u64,
    }

    impl Virtual {
        fn new() -> Self {
            Self {
                wall: i128::from(START),
                boot: "boot-a",
                with_suspend: 100,
                active: 100,
            }
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: WallTime::from_unix_nanos(self.wall * 1_000_000_000),
                boot: BootId(self.boot.to_owned()),
                since_boot_with_suspend: Duration::from_secs(self.with_suspend),
                since_boot_active: Duration::from_secs(self.active),
            }
        }

        fn run(&mut self, secs: u64) {
            self.wall += i128::from(secs);
            self.with_suspend += secs;
            self.active += secs;
        }

        fn suspend(&mut self, secs: u64) {
            self.wall += i128::from(secs);
            self.with_suspend += secs;
        }

        fn jump_wall(&mut self, secs: i128) {
            self.wall += secs;
        }

        fn reboot(&mut self, off_secs: u64, boot: &'static str) {
            self.wall += i128::from(off_secs);
            self.boot = boot;
            self.with_suspend = 5;
            self.active = 5;
        }
    }

    fn age_after(
        state: &mut AgeState,
        clock: &Virtual,
        kind: AgeClock,
        basis: &EntryTimeBasis,
    ) -> Duration {
        let now = clock.stamp();
        let age = AgeEvaluator::new(state, &now).age(kind, basis);
        state.observe(&now);
        age
    }

    fn committed(state: &mut AgeState, clock: &Virtual) -> EntryTimeBasis {
        let now = clock.stamp();
        state.observe(&now);
        EntryTimeBasis {
            committed: now.clone(),
            effective_wall: state.last_seen_wall.unwrap_or(now.wall),
            active_at_commit: state.active_accumulated,
        }
    }

    #[test]
    fn wall_age_counts_every_second_around_the_limit() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(29 * MINUTE + 59);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(29 * MINUTE + 59)
        );
        clock.run(1);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(30 * MINUTE)
        );
        clock.run(1);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(30 * MINUTE + 1)
        );
    }

    #[test]
    fn wall_clock_moved_back_does_not_extend_life() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(10 * MINUTE);
        clock.jump_wall(-i128::from(HOUR));
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(10 * MINUTE)
        );
        clock.run(21 * MINUTE);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(31 * MINUTE)
        );
    }

    #[test]
    fn wall_clock_moved_forward_shortens_life() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(5 * MINUTE);
        clock.jump_wall(i128::from(HOUR));
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(HOUR + 5 * MINUTE)
        );
    }

    #[test]
    fn wall_age_includes_suspend() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(MINUTE);
        clock.suspend(2 * HOUR);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(2 * HOUR + MINUTE)
        );
    }

    #[test]
    fn wall_age_includes_suspend_when_the_wall_clock_lags() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.suspend(2 * HOUR);
        clock.jump_wall(-2 * i128::from(HOUR));
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(2 * HOUR)
        );
    }

    #[test]
    fn wall_age_continues_after_restart() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(5 * MINUTE);
        age_after(&mut state, &clock, AgeClock::Wall, &basis);
        clock.reboot(35 * MINUTE, "boot-b");
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(40 * MINUTE)
        );
    }

    #[test]
    fn wall_age_survives_restart_with_clock_reset_backwards() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(20 * MINUTE);
        let before = age_after(&mut state, &clock, AgeClock::Wall, &basis);
        clock.reboot(0, "boot-b");
        clock.jump_wall(-i128::from(24 * HOUR));
        let after = age_after(&mut state, &clock, AgeClock::Wall, &basis);
        assert_eq!(before, Duration::from_secs(20 * MINUTE));
        assert!(after >= before);
    }

    #[test]
    fn wall_age_never_decreases() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        let steps: [(u64, i128, bool); 8] = [
            (60, 0, false),
            (60, -7200, false),
            (30, 3600, false),
            (0, -86_400, true),
            (90, 0, false),
            (0, -60, false),
            (10, 0, true),
            (600, -600, false),
        ];
        let mut previous = Duration::ZERO;
        for (index, (run, jump, reboot)) in steps.into_iter().enumerate() {
            if reboot {
                clock.reboot(0, if index % 2 == 0 { "boot-b" } else { "boot-c" });
            }
            clock.run(run);
            clock.jump_wall(jump);
            let age = age_after(&mut state, &clock, AgeClock::Wall, &basis);
            assert!(age >= previous, "step {index}: {age:?} < {previous:?}");
            previous = age;
        }
    }

    #[test]
    fn active_age_excludes_suspend() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(10 * MINUTE);
        clock.suspend(2 * HOUR);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Active, &basis),
            Duration::from_secs(10 * MINUTE)
        );
    }

    #[test]
    fn active_age_excludes_power_off_and_accumulates_across_boots() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(10 * MINUTE);
        age_after(&mut state, &clock, AgeClock::Active, &basis);
        clock.reboot(8 * HOUR, "boot-b");
        clock.run(5 * MINUTE);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Active, &basis),
            Duration::from_secs(15 * MINUTE + 5)
        );
        clock.reboot(HOUR, "boot-c");
        clock.run(MINUTE);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Active, &basis),
            Duration::from_secs(16 * MINUTE + 10)
        );
    }

    #[test]
    fn active_age_ignores_wall_clock_changes() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.run(3 * MINUTE);
        clock.jump_wall(i128::from(10 * HOUR));
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Active, &basis),
            Duration::from_secs(3 * MINUTE)
        );
    }

    #[test]
    fn entry_committed_later_is_younger() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let first = committed(&mut state, &clock);
        clock.run(MINUTE);
        let second = committed(&mut state, &clock);
        clock.run(MINUTE);
        let now = clock.stamp();
        let evaluator = AgeEvaluator::new(&state, &now);
        for kind in [AgeClock::Wall, AgeClock::Active] {
            assert_eq!(evaluator.age(kind, &first), Duration::from_secs(2 * MINUTE));
            assert_eq!(evaluator.age(kind, &second), Duration::from_secs(MINUTE));
        }
    }

    #[test]
    fn a_fresh_state_trusts_the_clock_readings() {
        let now = Virtual::new().stamp();
        let state = AgeState::default();
        assert_eq!(state.effective_wall(&now), now.wall);
        assert_eq!(state.active_total(&now), now.since_boot_active);
    }

    #[test]
    fn an_entry_of_another_boot_ages_by_the_wall_clock_only() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        let basis = committed(&mut state, &clock);
        clock.reboot(0, "boot-b");
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::ZERO
        );
        clock.run(MINUTE);
        assert_eq!(
            age_after(&mut state, &clock, AgeClock::Wall, &basis),
            Duration::from_secs(MINUTE)
        );
    }

    #[test]
    fn an_entry_committed_after_the_evaluation_has_no_age() {
        let mut recorded = AgeState::default();
        let late = Virtual::new();
        let basis = committed(&mut recorded, &late);
        let mut early = Virtual::new();
        early.jump_wall(-100);
        early.active -= 50;
        let now = early.stamp();
        let fresh = AgeState::default();
        for kind in [AgeClock::Wall, AgeClock::Active] {
            assert_eq!(
                AgeEvaluator::new(&fresh, &now).age(kind, &basis),
                Duration::ZERO
            );
        }
    }

    #[test]
    fn commit_basis_uses_the_effective_wall_time() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        state.observe(&clock.stamp());
        clock.run(MINUTE);
        clock.jump_wall(-i128::from(HOUR));
        let basis = committed(&mut state, &clock);
        assert_eq!(
            basis.effective_wall,
            WallTime::from_unix_nanos(i128::from(START + MINUTE) * 1_000_000_000)
        );
        assert_eq!(basis.committed, clock.stamp());
    }

    #[test]
    fn state_round_trips_through_its_file_format() {
        let mut state = AgeState::default();
        let mut clock = Virtual::new();
        state.observe(&clock.stamp());
        clock.run(MINUTE);
        state.observe(&clock.stamp());
        let bytes = state.encode().unwrap();
        assert_eq!(AgeState::decode(&bytes).unwrap(), state);
        let error = AgeState::decode(b"{\"last_seen_wall\": \"x\"}").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
