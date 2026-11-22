// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tombstones of expired entries.
//!
//! An expired entry leaves `tomb/<N>.json` in the journal directory of its
//! volume: the metrics shown by `history`, without data and without any
//! expiry time. At most `journal.history_keep` tombstones are kept per
//! volume; older ones are removed first.

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{EntryState, ExpiryReason, OpId, OpKind};
use nurt_contracts::report::Irreversible;
use serde::{Deserialize, Serialize};

use super::port::{Area, EntryFacts, RetentionStore};

/// Directory of tombstones in the journal directory of a volume.
pub(crate) const TOMB_DIR: &str = "tomb";

/// Width of the zero-padded operation number in file names.
const NUMBER_WIDTH: usize = 12;

/// Suffix of tombstone files.
const SUFFIX: &str = ".json";

/// Metrics of an expired entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Tombstone {
    /// Operation number.
    pub(crate) op: OpId,
    /// Commit time.
    pub(crate) time: WallTime,
    /// Kind of the operation.
    pub(crate) kind: OpKind,
    /// Number of items.
    pub(crate) files: u64,
    /// Logical bytes the entry held.
    pub(crate) bytes: u64,
    /// Irreversible reasons recorded for the operation.
    pub(crate) irreversible: Vec<Irreversible>,
    /// Always [`EntryState::Expired`].
    pub(crate) state: EntryState,
    /// Why the entry expired.
    pub(crate) reason: ExpiryReason,
}

impl Tombstone {
    /// Tombstone of an entry part that expired for `reason`.
    pub(crate) fn of(entry: &EntryFacts, reason: ExpiryReason) -> Self {
        Self {
            op: entry.op,
            time: entry.time,
            kind: entry.kind.clone(),
            files: entry.files,
            bytes: entry.bytes_held,
            irreversible: entry.irreversible.clone(),
            state: EntryState::Expired,
            reason,
        }
    }
}

/// Relative name of the tombstone of `op`.
pub(crate) fn file_name(op: OpId) -> String {
    format!("{TOMB_DIR}/{:0width$}{SUFFIX}", op.0, width = NUMBER_WIDTH)
}

/// Writes the tombstone of an entry on `volume`; idempotent.
///
/// # Errors
///
/// Returns `E_IO` when the file cannot be written.
pub(crate) fn write(
    store: &dyn RetentionStore,
    volume: &AbsPath,
    tombstone: &Tombstone,
) -> Result<()> {
    let content = serde_json::to_vec(tombstone).map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!(
                "cannot serialize the tombstone of entry {}: {error}",
                tombstone.op.0
            ),
        )
    })?;
    store.write_file(Area::Journal(volume), &file_name(tombstone.op), &content)
}

/// Tombstone of `op` on `volume`, when it exists.
///
/// # Errors
///
/// Returns `E_IO` when the file cannot be read and `E_JOURNAL_CORRUPT` when
/// it is not a valid tombstone.
pub(crate) fn read(
    store: &dyn RetentionStore,
    volume: &AbsPath,
    op: OpId,
) -> Result<Option<Tombstone>> {
    let Some(content) = store.read_file(Area::Journal(volume), &file_name(op))? else {
        return Ok(None);
    };
    serde_json::from_slice(&content).map(Some).map_err(|error| {
        Error::new(
            ErrorCode::JournalCorrupt,
            format!("tombstone of entry {} is not valid: {error}", op.0),
        )
    })
}

/// Numbers of the tombstones on `volume`, oldest first.
///
/// # Errors
///
/// Returns `E_IO` when the directory cannot be read.
pub(crate) fn list(store: &dyn RetentionStore, volume: &AbsPath) -> Result<Vec<OpId>> {
    let mut ops: Vec<OpId> = store
        .list_dir(Area::Journal(volume), TOMB_DIR)?
        .iter()
        .filter_map(|name| parse_name(name))
        .collect();
    ops.sort_unstable();
    Ok(ops)
}

/// Removes the oldest tombstones of `volume` so that at most `keep` remain;
/// returns the number removed.
///
/// # Errors
///
/// Returns `E_IO` when the directory cannot be read or a file cannot be
/// removed.
pub(crate) fn prune(store: &dyn RetentionStore, volume: &AbsPath, keep: u32) -> Result<usize> {
    let ops = list(store, volume)?;
    let keep = usize::try_from(keep).unwrap_or(usize::MAX);
    let excess = ops.len().saturating_sub(keep);
    for op in &ops[..excess] {
        store.remove_file(Area::Journal(volume), &file_name(*op))?;
    }
    Ok(excess)
}

fn parse_name(name: &str) -> Option<OpId> {
    let digits = name.strip_suffix(SUFFIX)?;
    if digits.len() != NUMBER_WIDTH || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(OpId)
}

#[cfg(test)]
mod tests {
    use super::super::testing::{MemStore, entry, mount};
    use super::*;

    #[test]
    fn tombstone_file_names_are_zero_padded() {
        assert_eq!(file_name(OpId(12)), "tomb/000000000012.json");
        assert_eq!(parse_name("000000000012.json"), Some(OpId(12)));
        assert_eq!(parse_name("12.json"), None);
        assert_eq!(parse_name("000000000012.json.tmp"), None);
        assert_eq!(parse_name("00000000001x.json"), None);
    }

    #[test]
    fn tombstone_round_trips_and_has_no_expiry_time() {
        let store = MemStore::new();
        let volume = mount("/home");
        let tombstone = Tombstone::of(&entry(7, "/home"), ExpiryReason::Ops);
        write(&store, &volume, &tombstone).unwrap();
        assert_eq!(read(&store, &volume, OpId(7)).unwrap(), Some(tombstone));
        assert_eq!(read(&store, &volume, OpId(8)).unwrap(), None);

        let text =
            String::from_utf8(store.file(Area::Journal(&volume), &file_name(OpId(7)))).unwrap();
        let fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).unwrap();
        let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "bytes",
                "files",
                "irreversible",
                "kind",
                "op",
                "reason",
                "state",
                "time"
            ]
        );
        assert_eq!(fields["state"], "expired");
        assert_eq!(fields["reason"], "ops");
    }

    #[test]
    fn invalid_tombstone_is_reported_as_corrupt() {
        let store = MemStore::new();
        let volume = mount("/home");
        store
            .write_file(Area::Journal(&volume), &file_name(OpId(3)), b"{}")
            .unwrap();
        let error = read(&store, &volume, OpId(3)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::JournalCorrupt);
    }

    #[test]
    fn history_keep_removes_the_oldest_tombstones() {
        let store = MemStore::new();
        let volume = mount("/home");
        for op in 1..=1005 {
            write(
                &store,
                &volume,
                &Tombstone::of(&entry(op, "/home"), ExpiryReason::Time),
            )
            .unwrap();
        }
        assert_eq!(prune(&store, &volume, 1000).unwrap(), 5);
        let left = list(&store, &volume).unwrap();
        assert_eq!(left.len(), 1000);
        assert_eq!(left.first(), Some(&OpId(6)));
        assert_eq!(left.last(), Some(&OpId(1005)));
        assert_eq!(prune(&store, &volume, 1000).unwrap(), 0);
    }
}
