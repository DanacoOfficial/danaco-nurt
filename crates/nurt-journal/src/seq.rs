// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Account-wide operation sequence.
//!
//! The file `seq` in the account state directory holds the last assigned
//! number as eight little-endian bytes followed by the CRC-32 of those bytes
//! in little-endian order. It is replaced atomically under the account lock.
//! When the file is missing or damaged, the sequence is rebuilt as the
//! largest number named by the account index or by any entry, temporary
//! entry, reclaimed entry, tombstone or marker of the known journals; a
//! number is never assigned twice.

use nurt_contracts::fs::{Durability, FsErrorKind, RenameMode, ScopePurpose};
use nurt_contracts::journal::OpId;
use nurt_fs::failpoint::FailPoint;

use crate::layout;
use crate::manifest::{self, ReadError};
use crate::store::{self, Fault, JournalCtx};

/// Length of the sequence file.
const SEQ_LEN: usize = 12;
/// Length of the number part of the sequence file.
const NUMBER_LEN: usize = 8;
/// Hint of the temporary file name of a sequence update.
const TEMP_HINT: &str = ".seq";

/// Content of the sequence file for `op`.
pub(crate) fn encode(op: OpId) -> [u8; SEQ_LEN] {
    let number = op.0.to_le_bytes();
    let crc = crc32fast::hash(&number).to_le_bytes();
    let mut bytes = [0_u8; SEQ_LEN];
    bytes[..NUMBER_LEN].copy_from_slice(&number);
    bytes[NUMBER_LEN..].copy_from_slice(&crc);
    bytes
}

/// Number recorded in a sequence file; `None` for a wrong length or a
/// checksum mismatch.
pub(crate) fn decode(bytes: &[u8]) -> Option<OpId> {
    let bytes: &[u8; SEQ_LEN] = bytes.try_into().ok()?;
    let (number, crc) = bytes.split_at(NUMBER_LEN);
    let number: [u8; NUMBER_LEN] = number.try_into().ok()?;
    let crc: [u8; SEQ_LEN - NUMBER_LEN] = crc.try_into().ok()?;
    (crc32fast::hash(&number) == u32::from_le_bytes(crc)).then(|| OpId(u64::from_le_bytes(number)))
}

/// Assigns the next operation number. The caller holds the account lock.
///
/// Fails when the sequence file cannot be read, rebuilt or replaced; the
/// caller then records the operation without a journal.
pub(crate) fn allocate(ctx: &JournalCtx) -> Result<OpId, Fault> {
    let path = ctx.state.seq()?;
    let current = match manifest::read_bytes(ctx.fs.as_ref(), &path) {
        Ok(bytes) => match decode(&bytes) {
            Some(op) => op,
            None => rebuild(ctx)?,
        },
        Err(error) if error.kind == FsErrorKind::NotFound => rebuild(ctx)?,
        Err(error) => return Err(error.into()),
    };
    ctx.hit(FailPoint::SeqBefore, None)?;
    let next = OpId(current.0.saturating_add(1));
    let scope = store::scope(next, ScopePurpose::Maintenance);
    let root = ctx.state.root();
    let mut handle = ctx.fs.create_temp_in(&scope, root, TEMP_HINT)?;
    let temp = handle.path().clone();
    let replaced = manifest::write_all(handle.as_mut(), &encode(next))
        .map_err(Fault::from)
        .and_then(|()| ctx.hit(FailPoint::SeqWritten, None))
        .and_then(|()| Ok(handle.sync(Durability::Full)?))
        .and_then(|()| {
            drop(handle);
            Ok(ctx.fs.rename(&scope, &temp, &path, RenameMode::Replace)?)
        });
    match replaced {
        Ok(()) => {}
        Err(Fault::Halt) => return Err(Fault::Halt),
        Err(fault) => {
            let _ = ctx.fs.remove_file(&scope, &temp);
            return Err(fault);
        }
    }
    ctx.fs.sync_dir(&scope, root)?;
    Ok(next)
}

/// Largest number named by the account index or by any known journal.
fn rebuild(ctx: &JournalCtx) -> Result<OpId, Fault> {
    let index = ctx.state.index()?;
    let rows = manifest::read_index(ctx.fs.as_ref(), &index).map_err(|error| match error {
        ReadError::Fs(error) => Fault::Fs(error),
        ReadError::Format(_) => Fault::Fs(nurt_contracts::fs::FsError::new(
            FsErrorKind::Io,
            "read_index",
            Some(index.as_path()),
        )),
    })?;
    let mut highest = rows.iter().map(|row| row.op).max().unwrap_or(OpId(0));
    for journal in ctx.active_journals() {
        if let Some(op) = layout::highest_number(ctx.fs.as_ref(), &journal)? {
            highest = highest.max(op);
        }
    }
    Ok(highest)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::fs::{FsError, FsWrite};
    use nurt_fs::failpoint::{FailAction, FailPlan, FailRule, SeededFailpoints};

    use super::*;
    use crate::manifest::IndexRow;
    use crate::store::test_support::Fixture;

    #[test]
    fn sequence_file_round_trips_with_checksum() {
        for op in [0, 1, 18, u64::MAX] {
            assert_eq!(decode(&encode(OpId(op))), Some(OpId(op)));
        }
        let mut damaged = encode(OpId(5));
        damaged[0] ^= 1;
        assert_eq!(decode(&damaged), None);
        assert_eq!(decode(&encode(OpId(5))[..11]), None);
    }

    #[test]
    fn assigns_consecutive_numbers_and_replaces_the_file_atomically() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        assert_eq!(allocate(&ctx), Ok(OpId(1)));
        assert_eq!(allocate(&ctx), Ok(OpId(2)));
        let bytes = manifest::read_bytes(ctx.fs.as_ref(), &ctx.state.seq().unwrap()).unwrap();
        assert_eq!(decode(&bytes), Some(OpId(2)));
        let log = fixture.fs.log();
        let rename = log.iter().position(|op| op.starts_with("rename ")).unwrap();
        let sync = log.iter().position(|op| op.starts_with("sync_file ")).unwrap();
        assert!(sync < rename, "{log:?}");
        assert!(log[rename..].iter().any(|op| op.starts_with("sync_dir ")));
    }

    #[test]
    fn damaged_sequence_is_rebuilt_without_reusing_numbers() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        let journal = &ctx.active_journals()[0];
        let scope = store::scope(OpId(1), ScopePurpose::Maintenance);
        fixture.fs.create_dir(&scope, &journal.tomb_dir().unwrap(), 0o700).unwrap();
        fixture.write(&journal.tomb(OpId(41)).unwrap(), b"{}");
        let row = IndexRow {
            op: OpId(39),
            time: nurt_contracts::env::WallTime(0),
            kind: nurt_contracts::journal::OpKind::command("remove"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: Vec::new(),
        };
        manifest::append_index_row(ctx.fs.as_ref(), &scope, &ctx.state.index().unwrap(), &row)
            .unwrap();
        fixture.write(&ctx.state.seq().unwrap(), b"garbage");
        assert_eq!(allocate(&ctx), Ok(OpId(42)));
        assert_eq!(allocate(&ctx), Ok(OpId(43)));
    }

    #[test]
    fn unreadable_rebuild_sources_forbid_assignment() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        fixture.write(&ctx.state.index().unwrap(), b"not json\n");
        assert!(matches!(allocate(&ctx), Err(Fault::Fs(_))));
        fixture.fs.fail("open_read", "seq", FsErrorKind::Io);
        assert!(matches!(allocate(&ctx), Err(Fault::Fs(_))));
    }

    #[test]
    fn injected_faults_stop_assignment_and_keep_the_previous_number() {
        for (point, action) in [
            (FailPoint::SeqBefore, FailAction::Error(FsErrorKind::Io)),
            (FailPoint::SeqWritten, FailAction::Error(FsErrorKind::NoSpace)),
            (FailPoint::SeqWritten, FailAction::Crash),
        ] {
            let fixture = Fixture::new();
            let mut ctx = fixture.ctx();
            assert_eq!(allocate(&ctx), Ok(OpId(1)));
            ctx.failpoints = Arc::new(SeededFailpoints::new(FailPlan::at(
                0,
                FailRule {
                    point,
                    n: None,
                    occurrence: 0,
                    action,
                },
            )));
            let expected = if action == FailAction::Crash {
                Err(Fault::Halt)
            } else {
                Err(Fault::Fs(FsError::new(FsErrorKind::Io, point.as_str(), None)))
            };
            let result = allocate(&ctx);
            if action == FailAction::Crash {
                assert_eq!(result, expected);
            } else {
                assert!(matches!(result, Err(Fault::Fs(_))), "{point:?}");
            }
            ctx.failpoints = Arc::new(nurt_fs::failpoint::NoFailpoints);
            assert_eq!(allocate(&ctx), Ok(OpId(2)), "{point:?}");
        }
    }

    #[test]
    fn failed_replacement_removes_the_temporary_file() {
        let fixture = Fixture::new();
        let ctx = fixture.ctx();
        fixture.fs.fail("rename", "seq", FsErrorKind::Io);
        assert!(allocate(&ctx).is_err());
        let names = fixture.names(ctx.state.root());
        assert!(names.iter().all(|name| !name.starts_with(TEMP_HINT)), "{names:?}");
    }

    #[test]
    fn concurrent_assignment_yields_unique_numbers() {
        let fixture = Fixture::new();
        let first = Arc::new(fixture.ctx());
        let second = Arc::new(fixture.ctx());
        let mut threads = Vec::new();
        for worker in 0..8 {
            let ctx = if worker % 2 == 0 { first.clone() } else { second.clone() };
            threads.push(std::thread::spawn(move || {
                (0..25)
                    .map(|_| {
                        let _lock = ctx.lock_account().unwrap();
                        allocate(&ctx).unwrap()
                    })
                    .collect::<Vec<_>>()
            }));
        }
        let mut numbers: Vec<u64> = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .map(|op| op.0)
            .collect();
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=200).collect::<Vec<_>>());
    }
}
