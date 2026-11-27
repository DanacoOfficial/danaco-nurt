// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes of a database through the operation journal.
//!
//! The order is fixed: the database file and its journal files are
//! preserved, the statements run (see [`Database::run_write`]), every
//! write-ahead log is checkpointed into its database file with `TRUNCATE`,
//! the connection is closed and the new state of the files is reported to
//! the journal. A database that the statements attach, and the file that
//! `VACUUM INTO` creates, are declared to the journal before the statement
//! that changes them. A failure rolls the SQLite transaction back and reports
//! nothing, so the executor rolls the journal transaction back and the files
//! return to their state before the call.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::report::{Change, ChangeAction, IrreversibleEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Record, Size};
use rusqlite::InterruptHandle;

use super::SqlRequest;
use std::path::PathBuf;

use nurt_contracts::report::Irreversible;

use super::journal::{DatabaseFiles, FileState};
use super::session::{self, Database, Files, Summary};
use super::{names, split, target};

/// Longest wait of the cancellation watcher before it checks the deadline.
const WATCH_INTERVAL: Duration = Duration::from_millis(20);

/// Result of a change.
#[derive(Debug)]
pub(crate) struct WriteOutcome {
    /// Rows returned by the statements, for example by `RETURNING`.
    pub(crate) rows: Vec<Record>,
    /// Change of the database file; `rows_changed` holds the number of rows
    /// inserted, updated or deleted.
    pub(crate) change: Change,
    /// Changes of the other database files of the call: attached databases
    /// and the files that `VACUUM INTO` creates.
    pub(crate) others: Vec<Change>,
    /// Files whose previous content the journal did not keep.
    pub(crate) irreversible: Vec<IrreversibleEntry>,
}

/// Planned changes shown by a preview: the database file, and the file of
/// every `ATTACH` and `VACUUM INTO` whose name is a literal, resolved like a
/// path argument. A file that is only known when the statement runs is listed
/// as an unknown file.
///
/// # Errors
///
/// Fails with `E_PATH_RELATIVE` for a relative file without a base.
pub(crate) fn planned_changes(request: &SqlRequest) -> Result<Vec<Change>> {
    let mut change = Change::new(request.display.clone(), ChangeAction::Modify);
    change.size_before = fs::metadata(&request.path)
        .ok()
        .map(|metadata| Size::from_bytes(metadata.len()));
    let mut changes = vec![change];
    for (index, text) in split::statement_texts(&request.query)
        .into_iter()
        .enumerate()
    {
        let (expression, creates) = match (target::attach_parts(text), target::vacuum_parts(text)) {
            (Some(parts), _) => (parts.expression, false),
            (None, Some(parts)) => (parts.expression, true),
            (None, None) => continue,
        };
        let literal = target::literal_of(&expression).map(|value| names::database_name(&value));
        match literal {
            Some(names::DatabaseName::Memory) => {}
            Some(names::DatabaseName::File { path, .. }) => {
                let file = names::absolute(request.base.as_deref(), &path)?;
                let action = if file.exists() {
                    ChangeAction::Modify
                } else {
                    ChangeAction::Create
                };
                changes.push(Change::new(file.to_string_lossy().into_owned(), action));
            }
            _ => changes.push(Change::new(
                format!("(file named by statement {})", index + 1),
                if creates {
                    ChangeAction::Create
                } else {
                    ChangeAction::Modify
                },
            )),
        }
    }
    Ok(changes)
}

/// Runs the statements as one change preserved by `tx`.
pub(crate) fn write(
    request: &SqlRequest,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> Result<WriteOutcome> {
    session::check_file(request)?;
    let mut journal = Journaled::new(tx, request);
    journal.declare(DatabaseFiles::locate(request)?)?;
    let mut rows = Vec::new();
    let summary = execute(request, ctx, &mut journal, &mut rows)?;
    let (size_before, size_after, others) = journal.finish(summary.wrote)?;
    let mut change = Change::new(request.display.clone(), ChangeAction::Modify);
    change.size_before = size_before.map(Size::from_bytes);
    change.size_after = size_after.map(Size::from_bytes);
    change.rows_changed = Some(summary.changes);
    Ok(WriteOutcome {
        rows,
        change,
        others,
        irreversible: journal.irreversible,
    })
}

/// Declarations of the database files of one change and their states before
/// it.
struct Journaled<'a> {
    tx: &'a mut dyn MutationTx,
    request: &'a SqlRequest,
    sets: Vec<Watched>,
    /// Index of the next declared target.
    next: u32,
    irreversible: Vec<IrreversibleEntry>,
}

/// One database with its side files, declared to the journal.
struct Watched {
    files: DatabaseFiles,
    before: Vec<FileState>,
    /// Index of the database file among the targets of the transaction.
    base: u32,
}

impl<'a> Journaled<'a> {
    fn new(tx: &'a mut dyn MutationTx, request: &'a SqlRequest) -> Self {
        Self {
            tx,
            request,
            sets: Vec::new(),
            next: 0,
            irreversible: Vec::new(),
        }
    }

    /// Preserves the files of a database that the journal does not know yet.
    fn declare(&mut self, files: DatabaseFiles) -> Result<()> {
        if self.sets.iter().any(|set| set.files.same_database(&files)) {
            return Ok(());
        }
        let before = files.snapshot();
        let preserved = self.tx.preserve(&files.specs(&before))?;
        self.irreversible.extend(files.irreversible(&preserved));
        let base = self.next;
        self.next += u32::try_from(before.len()).unwrap_or(u32::MAX);
        self.sets.push(Watched {
            files,
            before,
            base,
        });
        Ok(())
    }

    /// Reports the changes of every declared file to the journal and
    /// returns the size of the database of the call before and after, and
    /// the changes of the other database files.
    fn finish(&mut self, wrote: bool) -> Result<(Option<u64>, Option<u64>, Vec<Change>)> {
        let mut changes = Vec::new();
        let mut sizes = (None, None);
        let mut others = Vec::new();
        for (position, set) in self.sets.iter().enumerate() {
            let after = set.files.snapshot();
            let own = set
                .files
                .changes(&set.before, &after, wrote && position == 0, set.base);
            let changed = !own.is_empty();
            changes.extend(own);
            if position == 0 {
                sizes = (
                    set.before.first().map(FileState::len),
                    after.first().map(FileState::len),
                );
            } else if let (true, Some(old), Some(new)) =
                (changed, set.before.first(), after.first())
            {
                others.push(set.files.change_of(old, new));
            }
        }
        if !changes.is_empty() {
            self.tx.observe_external(&changes)?;
        }
        Ok((sizes.0, sizes.1, others))
    }
}

impl Files for Journaled<'_> {
    fn watch(&mut self, file: PathBuf, label: &str) -> Result<()> {
        let files = DatabaseFiles::at(&file, label, self.request)?;
        self.declare(files)
    }

    fn notes(&self) -> Vec<String> {
        self.irreversible
            .iter()
            .filter_map(|entry| entry.detail.clone())
            .collect()
    }

    fn untracked(&mut self, number: usize, what: &str) {
        let mut entry = IrreversibleEntry::new(Irreversible::ExternalEffect);
        entry.detail = Some(if number == 0 {
            what.to_owned()
        } else {
            format!("statement {number}: {what}")
        });
        self.irreversible.push(entry);
    }
}

fn execute(
    request: &SqlRequest,
    ctx: &ExecCtx,
    files: &mut dyn Files,
    rows: &mut Vec<Record>,
) -> Result<Summary> {
    let database = Database::open(request)?;
    let interrupt = database.connection().get_interrupt_handle();
    let finished = AtomicBool::new(false);
    let result = thread::scope(|scope| {
        scope.spawn(|| watch(ctx, &interrupt, &finished));
        let result = statements(&database, request, ctx, files, rows);
        finished.store(true, Ordering::Release);
        result
    });
    let effects = database.effects();
    let summary = result.map_err(|error| {
        let error = ctx.checkpoint().err().unwrap_or(error);
        let mut kept = files.notes();
        kept.extend(effects.describe());
        with_kept(error, kept)
    })?;
    for note in effects.describe() {
        files.untracked(0, &note);
    }
    database.close(request)?;
    Ok(summary)
}

/// Interrupts the running statement when the execution is cancelled or its
/// deadline passes.
///
/// An interrupt of SQLite reaches only the statements that run at that
/// moment, so a cancellation between two statements of the transaction
/// would be lost; the watcher repeats it until the transaction ends.
fn watch(ctx: &ExecCtx, interrupt: &InterruptHandle, finished: &AtomicBool) {
    while !finished.load(Ordering::Acquire) {
        let cancelled = ctx.cancel().wait_timeout(WATCH_INTERVAL).is_some();
        if cancelled || ctx.checkpoint().is_err() {
            interrupt.interrupt();
            thread::park_timeout(WATCH_INTERVAL);
        }
    }
}

/// Adds the effects that cannot be undone to the list of the error, keeping
/// the entries the error already has.
fn with_kept(error: Error, kept: Vec<String>) -> Error {
    let mut list: Vec<String> = error
        .details
        .get("irreversible")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    for note in kept {
        if !list.contains(&note) {
            list.push(note);
        }
    }
    if list.is_empty() {
        error
    } else {
        error.with_detail("irreversible", serde_json::json!(list))
    }
}

fn statements(
    database: &Database,
    request: &SqlRequest,
    ctx: &ExecCtx,
    files: &mut dyn Files,
    rows: &mut Vec<Record>,
) -> Result<Summary> {
    let max = ctx.limits().max_records;
    let mut sink = |record: Record| {
        if u64::try_from(rows.len()).unwrap_or(u64::MAX) >= max {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "the statements return more than {max} rows; return fewer rows from RETURNING or SELECT"
                ),
            ));
        }
        rows.push(record);
        Ok(())
    };
    database.run_write(request, files, &mut sink)
}
