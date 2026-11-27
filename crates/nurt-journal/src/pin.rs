// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::pin
// Responsibility:   Pinned entries (`journal.pin`).
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Pinned entries (`journal.pin`).
//!
//! A pin protects an entry from expiry by steps, age and size until a given
//! wall time; it never protects it from reclaim when the user needs space.
//! Pins are kept outside the manifest, in the pin file of the journal
//! directory of every volume that holds a part of the entry, so that pinning
//! changes neither the manifest hash nor the hash chain. Pinning creates no
//! entry and consumes no operation number and no step.
//!
//! The pin file is JSON: `{"v": 1, "crc32": C, "pins": [{"op": N, "until":
//! T}]}`, where `T` is the end of the pin in nanoseconds since the Unix epoch
//! and `C` is the CRC-32 (IEEE) of the serialized `pins` array. The end of a
//! pin is never part of a result.

use std::collections::BTreeMap;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{FsErrorKind, ScopePurpose, VolumeId};
use nurt_contracts::journal::{OpId, PinOutcome, PinRequest};
use serde::{Deserialize, Serialize};

use crate::history::EntrySource;
use crate::layout::JournalLayout;
use crate::manifest;
use crate::retention;
use crate::retention::ctx::CtxStore;
use crate::store::{self, JournalCtx};

/// Version of the pin file format.
pub(crate) const PIN_FILE_VERSION: u32 = 1;

/// Pin duration used when the request sets none (`journal.pin_max_age`).
pub(crate) const DEFAULT_PIN_MAX_AGE: Duration = Duration::from_hours(24);

/// Number of the scope of pin file writes, which belong to no entry.
const PIN_SCOPE_OP: OpId = OpId(0);

/// Pins of one volume, or of several volumes merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PinSet {
    until: BTreeMap<OpId, WallTime>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    v: u32,
    crc32: u32,
    pins: Vec<PinRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinRecord {
    op: OpId,
    until: WallTime,
}

impl PinSet {
    /// Parses a pin file.
    ///
    /// Fails with `E_IO` when the file is not a pin file of a known version
    /// or its checksum does not match.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let file: PinFile = serde_json::from_slice(bytes).map_err(|_| damaged())?;
        if file.v != PIN_FILE_VERSION {
            return Err(damaged().with_detail("version", serde_json::Value::from(file.v)));
        }
        if checksum(&file.pins)? != file.crc32 {
            return Err(damaged().with_problem("checksum"));
        }
        let until = file
            .pins
            .into_iter()
            .map(|record| (record.op, record.until))
            .collect();
        Ok(Self { until })
    }

    /// Serializes the pin file.
    ///
    /// Fails with `E_IO` only when serialization fails.
    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>> {
        let pins: Vec<PinRecord> = self
            .until
            .iter()
            .map(|(op, until)| PinRecord {
                op: *op,
                until: *until,
            })
            .collect();
        let file = PinFile {
            v: PIN_FILE_VERSION,
            crc32: checksum(&pins)?,
            pins,
        };
        serde_json::to_vec(&file).map_err(|_| damaged())
    }

    /// Whether the entry is pinned at `now`.
    pub(crate) fn is_pinned(&self, op: OpId, now: WallTime) -> bool {
        self.until.get(&op).is_some_and(|until| *until > now)
    }

    /// Pins an entry until the given time, replacing an earlier pin.
    pub(crate) fn pin(&mut self, op: OpId, until: WallTime) {
        self.until.insert(op, until);
    }

    /// Removes the pin of an entry.
    pub(crate) fn release(&mut self, op: OpId) {
        self.until.remove(&op);
    }

    /// Removes pins that ended at or before `now`.
    pub(crate) fn prune(&mut self, now: WallTime) {
        self.until.retain(|_, until| *until > now);
    }

    /// Adds the pins of another volume; the later end wins.
    pub(crate) fn merge(&mut self, other: &Self) {
        for (op, until) in &other.until {
            let slot = self.until.entry(*op).or_insert(*until);
            *slot = (*slot).max(*until);
        }
    }
}

fn checksum(pins: &[PinRecord]) -> Result<u32> {
    let bytes = serde_json::to_vec(pins).map_err(|_| damaged())?;
    Ok(crc32fast::hash(&bytes))
}

fn damaged() -> Error {
    Error::new(
        ErrorCode::Io,
        "the pin file of the journal is damaged; pin the entries again",
    )
}

/// Storage of pins and the journal services `journal.pin` relies on.
pub(crate) trait PinBackend: EntrySource {
    /// Pins of one volume; an absent pin file yields an empty set.
    fn read_pins(&self, volume: &VolumeId) -> Result<PinSet>;

    /// Replaces the pin file of one volume atomically.
    fn write_pins(&self, volume: &VolumeId, pins: &PinSet) -> Result<()>;

    /// Configured default pin duration (`journal.pin_max_age`).
    fn pin_max_age(&self) -> Duration;

    /// Expires every entry over a limit, now that a pin no longer protects
    /// it, and returns the entries the pass expired.
    fn expire_over_limits(&self) -> Result<Vec<OpId>>;
}

/// Pins of the journal directory of one volume; an absent or damaged pin
/// file yields the empty set, which the next pin replaces.
///
/// Fails with the file system error when the pin file cannot be read.
pub(crate) fn read_pin_file(ctx: &JournalCtx, journal: &JournalLayout) -> Result<PinSet> {
    match manifest::read_bytes(ctx.fs.as_ref(), &journal.pins()?) {
        Ok(bytes) => Ok(PinSet::parse(&bytes).unwrap_or_default()),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(PinSet::default()),
        Err(error) => Err(error.into()),
    }
}

/// Active journal of the volume, when there is one.
fn volume_journal(ctx: &JournalCtx, volume: &VolumeId) -> Result<Option<JournalLayout>> {
    for journal in ctx.active_journals() {
        if &ctx.fs.volume(journal.root())?.id == volume {
            return Ok(Some(journal));
        }
    }
    Ok(None)
}

/// Pin files in the journal directories of the journal store. The default
/// pin duration is `journal.pin_max_age`, or [`DEFAULT_PIN_MAX_AGE`] when the
/// configuration is invalid; releasing runs an expiry pass.
impl PinBackend for JournalCtx {
    fn read_pins(&self, volume: &VolumeId) -> Result<PinSet> {
        match volume_journal(self, volume)? {
            Some(journal) => read_pin_file(self, &journal),
            None => Ok(PinSet::default()),
        }
    }

    fn write_pins(&self, volume: &VolumeId, pins: &PinSet) -> Result<()> {
        let journal = volume_journal(self, volume)?.ok_or_else(|| {
            Error::new(
                ErrorCode::Io,
                "the journal of a volume of the entry is not available; mount the volume and pin again",
            )
        })?;
        let scope = store::scope(PIN_SCOPE_OP, ScopePurpose::Maintenance);
        manifest::write_atomic(
            self.fs.as_ref(),
            &scope,
            journal.root(),
            &journal.pins()?,
            &pins.to_bytes()?,
        )?;
        Ok(())
    }

    fn pin_max_age(&self) -> Duration {
        retention::expiry::resolve_config(&CtxStore(self)).map_or(DEFAULT_PIN_MAX_AGE, |resolved| {
            resolved.config.journal.pin_max_age.as_std()
        })
    }

    fn expire_over_limits(&self) -> Result<Vec<OpId>> {
        let report = retention::expiry::sweep(self)?;
        Ok(report.expired.into_iter().map(|entry| entry.op).collect())
    }
}

/// Pins or releases entries of the journal of the account.
///
/// Fails as [`pin_with`].
pub(crate) fn pin(ctx: &JournalCtx, request: &PinRequest) -> Result<PinOutcome> {
    pin_with(ctx, request)
}

/// Pins or releases entries.
///
/// Entries that are not available (unknown, expired, undone or in a
/// transient state) are reported in `not_available` and left alone.
/// Releasing pins expires at once the entries that are over a limit; the
/// released entries among them are reported in `expired`. Entries the same
/// pass expires without a released pin are not reported, because the release
/// did not change their limits.
///
/// Fails with `E_IO` when a pin file cannot be read or written.
pub(crate) fn pin_with<S>(store: &S, request: &PinRequest) -> Result<PinOutcome>
where
    S: PinBackend + ?Sized,
{
    let now = store.now();
    let until = now.saturating_add(request.until.unwrap_or_else(|| store.pin_max_age()));
    let mut changed = Vec::new();
    let mut not_available = Vec::new();
    let mut by_volume: BTreeMap<VolumeId, Vec<OpId>> = BTreeMap::new();
    for &op in &request.ids {
        if changed.contains(&op) || not_available.contains(&op) {
            continue;
        }
        match store.load(op)? {
            Some(entry) if entry.is_available() && !entry.volumes.is_empty() => {
                for volume in entry.volumes {
                    by_volume.entry(volume).or_default().push(op);
                }
                changed.push(op);
            }
            _ => not_available.push(op),
        }
    }
    for (volume, ops) in &by_volume {
        let mut pins = store.read_pins(volume)?;
        pins.prune(now);
        for &op in ops {
            if request.release {
                pins.release(op);
            } else {
                pins.pin(op, until);
            }
        }
        store.write_pins(volume, &pins)?;
    }
    let mut expired = Vec::new();
    if request.release && !changed.is_empty() {
        expired = store.expire_over_limits()?;
        expired.retain(|op| changed.contains(op));
    }
    Ok(PinOutcome {
        changed,
        not_available,
        expired,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::WallTime;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::{EntryState, OpId, PinRequest};

    use super::{PinSet, pin_with as pin};
    use crate::undo::testkit::{TestJournal, volume};

    fn request(ids: &[u64], release: bool, until: Option<Duration>) -> PinRequest {
        PinRequest {
            ids: ids.iter().map(|&n| OpId(n)).collect(),
            release,
            until,
        }
    }

    #[test]
    fn pin_file_round_trips_and_detects_damage() {
        let mut pins = PinSet::default();
        pins.pin(OpId(3), WallTime::from_unix_nanos(10));
        pins.pin(OpId(9), WallTime::from_unix_nanos(20));
        let bytes = pins.to_bytes().unwrap();
        assert_eq!(PinSet::parse(&bytes).unwrap(), pins);
        let text = String::from_utf8(bytes).unwrap();
        let tampered = text.replace("\"until\":20", "\"until\":21");
        assert_eq!(
            PinSet::parse(tampered.as_bytes()).unwrap_err().code,
            ErrorCode::Io
        );
        let future = text.replace("\"v\":1", "\"v\":2");
        assert!(PinSet::parse(future.as_bytes()).is_err());
        assert!(PinSet::parse(b"[]").is_err());
    }

    #[test]
    fn pin_set_prunes_ended_pins_and_merges_by_latest_end() {
        let mut first = PinSet::default();
        first.pin(OpId(1), WallTime::from_unix_nanos(5));
        first.pin(OpId(2), WallTime::from_unix_nanos(50));
        let mut second = PinSet::default();
        second.pin(OpId(2), WallTime::from_unix_nanos(80));
        first.merge(&second);
        assert!(first.is_pinned(OpId(2), WallTime::from_unix_nanos(70)));
        first.prune(WallTime::from_unix_nanos(5));
        assert!(!first.is_pinned(OpId(1), WallTime::from_unix_nanos(0)));
    }

    #[test]
    fn pin_writes_pin_files_outside_the_manifest() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        let manifest = journal.entry(OpId(1)).manifest_hash;
        let seq = journal.seq();
        let outcome = pin(&journal, &request(&[1], false, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(1)]);
        assert!(outcome.not_available.is_empty());
        assert_eq!(journal.entry(OpId(1)).manifest_hash, manifest);
        assert_eq!(journal.seq(), seq);
        assert!(
            journal
                .pins_of(&volume(1))
                .is_pinned(OpId(1), journal.now_time())
        );
    }

    #[test]
    fn pin_uses_requested_duration_or_configured_default() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        pin(
            &journal,
            &request(&[1], false, Some(Duration::from_mins(1))),
        )
        .unwrap();
        pin(&journal, &request(&[2], false, None)).unwrap();
        journal.advance(Duration::from_secs(61));
        let pins = journal.pins_of(&volume(1));
        assert!(!pins.is_pinned(OpId(1), journal.now_time()));
        assert!(pins.is_pinned(OpId(2), journal.now_time()));
        journal.advance(Duration::from_hours(24));
        assert!(
            !journal
                .pins_of(&volume(1))
                .is_pinned(OpId(2), journal.now_time())
        );
    }

    #[test]
    fn pin_protects_from_step_limit_until_release_expires_entry() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        pin(&journal, &request(&[1], false, None)).unwrap();
        for n in 0..3 {
            journal
                .op(None)
                .create(&format!("/home/user/n{n}"), b"n")
                .commit();
        }
        journal.set_step_limit(2);
        journal.sweep();
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Committed);
        let outcome = pin(&journal, &request(&[1], true, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(1)]);
        assert_eq!(outcome.expired, [OpId(1)]);
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Expired);
    }

    #[test]
    fn pin_release_reports_only_released_entries_that_expired() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.op(None).create("/home/user/c", b"c").commit();
        pin(&journal, &request(&[1, 3], false, None)).unwrap();
        for n in 0..2 {
            journal
                .op(None)
                .create(&format!("/home/user/n{n}"), b"n")
                .commit();
        }
        journal.set_step_limit(3);
        let outcome = pin(&journal, &request(&[1, 3], true, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(1), OpId(3)]);
        assert_eq!(outcome.expired, [OpId(1)]);
        assert_eq!(journal.entry(OpId(2)).state, EntryState::Expired);
        assert_eq!(journal.entry(OpId(3)).state, EntryState::Committed);
    }

    #[test]
    fn pin_without_release_expires_nothing() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.set_step_limit(1);
        let outcome = pin(&journal, &request(&[1], false, None)).unwrap();
        assert!(outcome.expired.is_empty());
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Committed);
    }

    #[test]
    fn pin_reports_unavailable_entries() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        journal.op(None).create("/home/user/c", b"c").commit();
        journal.expire(OpId(1));
        journal.set_state(OpId(2), EntryState::Undone);
        let outcome = pin(&journal, &request(&[1, 2, 3, 3, 9], false, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(3)]);
        assert_eq!(outcome.not_available, [OpId(1), OpId(2), OpId(9)]);
    }

    #[test]
    fn pin_marks_every_volume_of_a_multi_volume_entry() {
        let journal = TestJournal::new();
        journal
            .op(None)
            .create("/home/user/a", b"a")
            .on_volumes(&[1, 2])
            .commit();
        pin(&journal, &request(&[1], false, None)).unwrap();
        let now = journal.now_time();
        assert!(journal.pins_of(&volume(1)).is_pinned(OpId(1), now));
        assert!(journal.pins_of(&volume(2)).is_pinned(OpId(1), now));
        pin(&journal, &request(&[1], true, None)).unwrap();
        assert!(!journal.pins_of(&volume(2)).is_pinned(OpId(1), now));
    }

    #[test]
    fn pin_ends_exactly_at_its_end_time() {
        let mut pins = PinSet::default();
        pins.pin(OpId(1), WallTime::from_unix_nanos(10));
        assert!(pins.is_pinned(OpId(1), WallTime::from_unix_nanos(9)));
        assert!(!pins.is_pinned(OpId(1), WallTime::from_unix_nanos(10)));
        assert!(!pins.is_pinned(OpId(2), WallTime::from_unix_nanos(0)));
        pins.prune(WallTime::from_unix_nanos(9));
        assert!(pins.is_pinned(OpId(1), WallTime::from_unix_nanos(9)));
        pins.prune(WallTime::from_unix_nanos(10));
        assert_eq!(pins, PinSet::default());
    }

    #[test]
    fn pinning_again_replaces_the_pin_and_releasing_an_unpinned_entry_changes_nothing() {
        let mut pins = PinSet::default();
        pins.pin(OpId(3), WallTime::from_unix_nanos(100));
        pins.pin(OpId(3), WallTime::from_unix_nanos(50));
        assert!(!pins.is_pinned(OpId(3), WallTime::from_unix_nanos(50)));
        assert!(pins.is_pinned(OpId(3), WallTime::from_unix_nanos(49)));
        pins.release(OpId(9));
        assert!(pins.is_pinned(OpId(3), WallTime::from_unix_nanos(49)));
        pins.release(OpId(3));
        assert_eq!(pins, PinSet::default());
    }

    #[test]
    fn empty_pin_files_round_trip_and_damaged_ones_are_rejected() {
        let empty = PinSet::default();
        assert_eq!(PinSet::parse(&empty.to_bytes().unwrap()).unwrap(), empty);
        for damaged in [
            &b""[..],
            b"\xff\xfe",
            b"{}",
            b"null",
            b"{\"v\":1,\"crc32\":0,\"pins\":[],\"extra\":1}",
            b"{\"v\":1,\"crc32\":0,\"pins\":[{\"op\":1}]}",
            b"{\"v\":-1,\"crc32\":0,\"pins\":[]}",
        ] {
            assert_eq!(
                PinSet::parse(damaged).unwrap_err().code,
                ErrorCode::Io,
                "{damaged:?}"
            );
        }
    }

    #[test]
    fn merge_with_an_empty_set_and_of_disjoint_entries_keeps_every_pin() {
        let mut first = PinSet::default();
        first.pin(OpId(1), WallTime::from_unix_nanos(10));
        let mut second = PinSet::default();
        second.pin(OpId(2), WallTime::from_unix_nanos(20));
        first.merge(&PinSet::default());
        first.merge(&second);
        assert!(first.is_pinned(OpId(1), WallTime::from_unix_nanos(0)));
        assert!(first.is_pinned(OpId(2), WallTime::from_unix_nanos(19)));
        let mut earlier = PinSet::default();
        earlier.pin(OpId(2), WallTime::from_unix_nanos(5));
        first.merge(&earlier);
        assert!(first.is_pinned(OpId(2), WallTime::from_unix_nanos(19)));
    }

    #[test]
    fn pinning_nothing_or_releasing_unknown_entries_changes_and_expires_nothing() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        let nothing = pin(&journal, &request(&[], false, None)).unwrap();
        assert!(nothing.changed.is_empty());
        assert!(nothing.not_available.is_empty());
        assert!(nothing.expired.is_empty());
        let unknown = pin(&journal, &request(&[9, 9], true, None)).unwrap();
        assert!(unknown.changed.is_empty());
        assert_eq!(unknown.not_available, [OpId(9)]);
        assert!(unknown.expired.is_empty());
    }

    #[test]
    fn releasing_a_pin_stops_the_protection_at_once() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        pin(&journal, &request(&[1], false, None)).unwrap();
        let outcome = pin(&journal, &request(&[1], true, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(1)]);
        assert!(outcome.expired.is_empty());
        assert!(
            !journal
                .pins_of(&volume(1))
                .is_pinned(OpId(1), journal.now_time())
        );
        let again = pin(&journal, &request(&[1], true, None)).unwrap();
        assert_eq!(again.changed, [OpId(1)]);
    }

    #[test]
    fn pin_results_carry_no_end_of_pin() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        let outcome = pin(&journal, &request(&[1], false, None)).unwrap();
        let json = serde_json::to_value(&outcome).unwrap();
        let mut fields: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["changed", "expired", "not_available"]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
