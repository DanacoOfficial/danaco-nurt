// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "entry points are called by the journal store")
)]

use std::collections::BTreeMap;
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::VolumeId;
use nurt_contracts::journal::{OpId, PinOutcome, PinRequest};
use serde::{Deserialize, Serialize};

use crate::history::EntrySource;

/// Version of the pin file format.
pub(crate) const PIN_FILE_VERSION: u32 = 1;

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

    /// Appends the audit record of a pin or release to the journal index.
    fn record_pin(&self, op: OpId, release: bool) -> Result<()>;

    /// Configured default pin duration (`journal.pin_max_age`).
    fn pin_max_age(&self) -> Duration;

    /// Expires every entry over a limit, now that a pin no longer protects
    /// it.
    fn expire_over_limits(&self) -> Result<()>;
}

/// Pins or releases entries.
///
/// Entries that are not available (unknown, expired, undone or in a
/// transient state) are reported in `not_available` and left alone.
/// Releasing pins expires at once the entries that are over a limit.
///
/// Fails with `E_IO` when a pin file cannot be read or written.
pub(crate) fn pin<S>(store: &S, request: &PinRequest) -> Result<PinOutcome>
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
    for &op in &changed {
        store.record_pin(op, request.release)?;
    }
    if request.release && !changed.is_empty() {
        store.expire_over_limits()?;
    }
    Ok(PinOutcome {
        changed,
        not_available,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::WallTime;
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::journal::{EntryState, OpId, PinRequest};

    use super::{PinSet, pin};
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
        assert_eq!(PinSet::parse(tampered.as_bytes()).unwrap_err().code, ErrorCode::Io);
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
        assert!(journal.pins_of(&volume(1)).is_pinned(OpId(1), journal.now_time()));
        assert_eq!(journal.audit(), [(OpId(1), false)]);
    }

    #[test]
    fn pin_uses_requested_duration_or_configured_default() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        journal.op(None).create("/home/user/b", b"b").commit();
        pin(&journal, &request(&[1], false, Some(Duration::from_secs(60)))).unwrap();
        pin(&journal, &request(&[2], false, None)).unwrap();
        journal.advance(Duration::from_secs(61));
        let pins = journal.pins_of(&volume(1));
        assert!(!pins.is_pinned(OpId(1), journal.now_time()));
        assert!(pins.is_pinned(OpId(2), journal.now_time()));
        journal.advance(Duration::from_secs(24 * 3600));
        assert!(!journal.pins_of(&volume(1)).is_pinned(OpId(2), journal.now_time()));
    }

    #[test]
    fn pin_protects_from_step_limit_until_release_expires_entry() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        pin(&journal, &request(&[1], false, None)).unwrap();
        for n in 0..3 {
            journal.op(None).create(&format!("/home/user/n{n}"), b"n").commit();
        }
        journal.set_step_limit(2);
        journal.sweep();
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Committed);
        let outcome = pin(&journal, &request(&[1], true, None)).unwrap();
        assert_eq!(outcome.changed, [OpId(1)]);
        assert_eq!(journal.entry(OpId(1)).state, EntryState::Expired);
        assert_eq!(journal.audit(), [(OpId(1), false), (OpId(1), true)]);
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
        journal.op(None).create("/home/user/a", b"a").on_volumes(&[1, 2]).commit();
        pin(&journal, &request(&[1], false, None)).unwrap();
        let now = journal.now_time();
        assert!(journal.pins_of(&volume(1)).is_pinned(OpId(1), now));
        assert!(journal.pins_of(&volume(2)).is_pinned(OpId(1), now));
        pin(&journal, &request(&[1], true, None)).unwrap();
        assert!(!journal.pins_of(&volume(2)).is_pinned(OpId(1), now));
    }

    #[test]
    fn pin_results_carry_no_end_of_pin() {
        let journal = TestJournal::new();
        journal.op(None).create("/home/user/a", b"a").commit();
        let outcome = pin(&journal, &request(&[1], false, None)).unwrap();
        let json = serde_json::to_value(&outcome).unwrap();
        let fields: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(fields, ["changed", "not_available"]);
    }
}
