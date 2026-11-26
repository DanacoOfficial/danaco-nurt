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

/// Extension of the file that reserves the identifier of an entry that is
/// not saved yet.
const RESERVE_EXTENSION: &str = "rsv";

/// Largest count of numbers tried when an identifier is reserved.
const RESERVE_ATTEMPTS: u32 = 1_000;

/// Directory of the files the test frameworks write for a run, inside the
/// history directory.
const REPORTS_DIR_NAME: &str = "reports";

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

    fn reservation(&self, kind: EntryKind, id: &str) -> PathBuf {
        self.directory(kind)
            .join(format!("{id}.{RESERVE_EXTENSION}"))
    }

    /// Sequence numbers of the stored entries, ascending.
    fn numbers(&self, kind: EntryKind) -> Vec<u64> {
        self.numbers_with(kind, ENTRY_EXTENSION)
    }

    /// Sequence numbers of the files with the extension, ascending.
    fn numbers_with(&self, kind: EntryKind, extension: &str) -> Vec<u64> {
        let Ok(entries) = fs::read_dir(self.directory(kind)) else {
            return Vec::new();
        };
        let mut numbers: Vec<u64> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = name.strip_suffix(&format!(".{extension}"))?;
                stem.strip_prefix(prefix(kind))?.parse().ok()
            })
            .collect();
        numbers.sort_unstable();
        numbers
    }

    /// Reserves the next identifier: a run that lasts minutes holds its
    /// identifier until it is saved, so a run that starts meanwhile gets
    /// another one.
    pub(crate) fn next_id(&self, kind: EntryKind) -> String {
        let directory = self.directory(kind);
        let _ = fs::create_dir_all(&directory);
        let mut next = self
            .numbers(kind)
            .into_iter()
            .chain(self.numbers_with(kind, RESERVE_EXTENSION))
            .max()
            .map_or(1, |last| last.saturating_add(1));
        for _ in 0..RESERVE_ATTEMPTS {
            let reserved = self.reservation(kind, &format!("{}{next}", prefix(kind)));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(reserved)
            {
                Err(failure) if failure.kind() == std::io::ErrorKind::AlreadyExists => {
                    next = next.saturating_add(1);
                }
                _ => break,
            }
        }
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
            Error::new(
                ErrorCode::Internal,
                format!("cannot encode the entry: {failure}"),
            )
        })?;
        let target = self.file(kind, &entry.id);
        let temporary = target.with_extension(TEMP_EXTENSION);
        let written =
            write_synced(&temporary, &text).and_then(|()| fs::rename(&temporary, &target));
        if let Err(failure) = written {
            let _ = fs::remove_file(&temporary);
            return Err(io_error(failure));
        }
        let _ = fs::remove_file(self.reservation(kind, &entry.id));
        self.prune(kind);
        Ok(())
    }

    /// Removes the oldest entries beyond the history size, with the files the
    /// frameworks wrote for them, and the reservations that are behind the
    /// oldest entry. An entry that cannot be removed stays; the next save
    /// tries again.
    fn prune(&self, kind: EntryKind) {
        let numbers = self.numbers(kind);
        let excess = numbers.len().saturating_sub(self.history_max);
        for number in numbers.iter().copied().take(excess) {
            let id = format!("{}{number}", prefix(kind));
            let _ = fs::remove_file(self.file(kind, &id));
            if kind == EntryKind::TestRun {
                self.remove_reports(&id);
            }
        }
        if let Some(oldest) = numbers.get(excess).copied() {
            for number in self.numbers_with(kind, RESERVE_EXTENSION) {
                if number < oldest {
                    let id = format!("{}{number}", prefix(kind));
                    let _ = fs::remove_file(self.reservation(kind, &id));
                }
            }
        }
    }

    /// Removes the report and the coverage the frameworks wrote for a run.
    fn remove_reports(&self, id: &str) {
        let reports = self.root.join(REPORTS_DIR_NAME);
        for extension in ["xml", "json", "coverage"] {
            let _ = fs::remove_file(reports.join(format!("{id}.{extension}")));
        }
        let _ = fs::remove_dir_all(reports.join(format!("{id}-coverage")));
    }

    pub(crate) fn load(&self, kind: EntryKind, id: &str) -> Result<Option<StoredEntry>> {
        if !is_identifier(kind, id) {
            return Ok(None);
        }
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

/// Whether `id` is the prefix of the kind followed by a number; any other
/// text would name a file outside the history directory.
fn is_identifier(kind: EntryKind, id: &str) -> bool {
    id.strip_prefix(prefix(kind)).is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Writes the bytes to a new file and flushes them to the disk.
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn io_error(failure: std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot access the run history: {failure}"),
    )
}

/// Store that resolves its directory and size from the execution context.
struct ContextStore;

fn directory_store(ctx: &ExecCtx) -> Result<DirStore> {
    let state = ctx.state_dir().ok_or_else(|| {
        Error::new(
            ErrorCode::Unsupported,
            "the state directory is not configured",
        )
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
