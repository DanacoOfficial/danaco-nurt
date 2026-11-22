// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! History of test runs and analyses in the state directory.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Record;
use serde::{Deserialize, Serialize};

use crate::iface::{EntryKind, StoredEntry, VerifyStore};

/// Directory of the history inside the state directory.
const STORE_DIR: &str = "verify";

/// Extension of a stored entry.
const ENTRY_EXTENSION: &str = "json";

/// Extension of an entry being written.
const TEMP_EXTENSION: &str = "tmp";

/// Entries are kept in files named from the identifier prefix of the kind
/// and a sequence number.
fn prefix(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::TestRun => "run-",
        EntryKind::Analysis => "an-",
    }
}

fn subdirectory(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::TestRun => "runs",
        EntryKind::Analysis => "analyses",
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    header: Record,
    items: Vec<Record>,
}

/// History kept in a directory.
#[derive(Debug, Clone)]
pub(crate) struct DirStore {
    root: PathBuf,
    history_max: usize,
}

impl DirStore {
    /// History under `state_dir`, keeping no more than `history_max`
    /// entries of each kind.
    pub(crate) fn new(state_dir: &Path, history_max: u32) -> Self {
        Self {
            root: state_dir.join(STORE_DIR),
            history_max: usize::try_from(history_max).unwrap_or(usize::MAX).max(1),
        }
    }

    fn directory(&self, kind: EntryKind) -> PathBuf {
        self.root.join(subdirectory(kind))
    }

    fn file(&self, kind: EntryKind, id: &str) -> PathBuf {
        self.directory(kind).join(format!("{id}.{ENTRY_EXTENSION}"))
    }

    /// Sequence numbers of the stored entries, ascending.
    fn numbers(&self, kind: EntryKind) -> Vec<u64> {
        let Ok(entries) = fs::read_dir(self.directory(kind)) else {
            return Vec::new();
        };
        let mut numbers: Vec<u64> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = name.strip_suffix(&format!(".{ENTRY_EXTENSION}"))?;
                stem.strip_prefix(prefix(kind))?.parse().ok()
            })
            .collect();
        numbers.sort_unstable();
        numbers
    }

    pub(crate) fn next_id(&self, kind: EntryKind) -> String {
        let next = self.numbers(kind).last().map_or(1, |last| last + 1);
        format!("{}{next}", prefix(kind))
    }

    pub(crate) fn save(&self, kind: EntryKind, entry: &StoredEntry) -> Result<()> {
        let directory = self.directory(kind);
        fs::create_dir_all(&directory).map_err(io_error)?;
        let stored = Stored {
            header: entry.header.clone(),
            items: entry.items.clone(),
        };
        let text = serde_json::to_vec(&stored).map_err(|failure| {
            Error::new(ErrorCode::Internal, format!("cannot encode the entry: {failure}"))
        })?;
        let target = self.file(kind, &entry.id);
        let temporary = target.with_extension(TEMP_EXTENSION);
        let mut file = fs::File::create(&temporary).map_err(io_error)?;
        file.write_all(&text).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, &target).map_err(io_error)?;
        self.prune(kind);
        Ok(())
    }

    /// Removes the oldest entries beyond the history size. An entry that
    /// cannot be removed stays; the next save tries again.
    fn prune(&self, kind: EntryKind) {
        let numbers = self.numbers(kind);
        let excess = numbers.len().saturating_sub(self.history_max);
        for number in numbers.into_iter().take(excess) {
            let id = format!("{}{number}", prefix(kind));
            let _ = fs::remove_file(self.file(kind, &id));
        }
    }

    pub(crate) fn load(&self, kind: EntryKind, id: &str) -> Result<Option<StoredEntry>> {
        let path = self.file(kind, id);
        let text = match fs::read(&path) {
            Ok(text) => text,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(failure) => return Err(io_error(failure)),
        };
        let stored: Stored = serde_json::from_slice(&text).map_err(|failure| {
            Error::new(
                ErrorCode::Io,
                format!("the stored entry {id} is not readable: {failure}"),
            )
            .with_fix("remove the entry file from the history directory")
        })?;
        Ok(Some(StoredEntry {
            id: id.to_owned(),
            header: stored.header,
            items: stored.items,
        }))
    }

    pub(crate) fn list(&self, kind: EntryKind, limit: u32) -> Result<Vec<Record>> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut headers = Vec::new();
        for number in self.numbers(kind).into_iter().rev().take(limit) {
            let id = format!("{}{number}", prefix(kind));
            if let Some(entry) = self.load(kind, &id)? {
                headers.push(entry.header);
            }
        }
        Ok(headers)
    }

    pub(crate) fn ids(&self, kind: EntryKind) -> Vec<String> {
        self.numbers(kind)
            .into_iter()
            .rev()
            .map(|number| format!("{}{number}", prefix(kind)))
            .collect()
    }
}

fn io_error(failure: std::io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("cannot access the run history: {failure}"))
}

/// Store that resolves its directory and size from the execution context.
struct ContextStore;

fn directory_store(ctx: &ExecCtx) -> Result<DirStore> {
    let state = ctx.state_dir().ok_or_else(|| {
        Error::new(ErrorCode::Unsupported, "the state directory is not configured")
            .with_fix("set state.dir")
    })?;
    Ok(DirStore::new(state, ctx.config().verify.history_max))
}

impl VerifyStore for ContextStore {
    fn next_id(&self, ctx: &ExecCtx, kind: EntryKind) -> Result<String> {
        Ok(directory_store(ctx)?.next_id(kind))
    }

    fn save(&self, ctx: &ExecCtx, kind: EntryKind, entry: &StoredEntry) -> Result<()> {
        directory_store(ctx)?.save(kind, entry)
    }

    fn load(&self, ctx: &ExecCtx, kind: EntryKind, id: &str) -> Result<Option<StoredEntry>> {
        directory_store(ctx)?.load(kind, id)
    }

    fn list(&self, ctx: &ExecCtx, kind: EntryKind, limit: u32) -> Result<Vec<Record>> {
        directory_store(ctx)?.list(kind, limit)
    }

    fn ids(&self, ctx: &ExecCtx, kind: EntryKind) -> Result<Vec<String>> {
        Ok(directory_store(ctx)?.ids(kind))
    }
}

pub(crate) fn verify_store() -> Arc<dyn VerifyStore> {
    Arc::new(ContextStore)
}

#[cfg(test)]
mod tests;
