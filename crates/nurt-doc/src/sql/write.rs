// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes of a database through the operation journal.
//!
//! The order is fixed: the database file and its journal files are
//! preserved, the statements run in one `BEGIN IMMEDIATE` transaction, the
//! transaction is committed, the write-ahead log is checkpointed into the
//! database file with `TRUNCATE`, the connection is closed and the new state
//! of the files is reported to the journal. A failure before the commit
//! rolls the SQLite transaction back and reports nothing, so the executor
//! rolls the journal transaction back and no entry remains.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::report::{Change, ChangeAction, IrreversibleEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Record, Size};
use rusqlite::{Connection, InterruptHandle};

use super::SqlRequest;
use super::error::{self, Site};
use super::journal::DatabaseFiles;
use super::session::{self, Summary};

/// Longest wait of the cancellation watcher before it checks the deadline.
const WATCH_INTERVAL: Duration = Duration::from_millis(20);

/// Result of a change.
#[derive(Debug)]
pub(crate) struct WriteOutcome {
    /// Rows returned by the statements, for example by `RETURNING`.
    pub(crate) rows: Vec<Record>,
    /// Change of the database file; `replacements` holds the number of rows
    /// inserted, updated or deleted.
    pub(crate) change: Change,
    /// Files whose previous content the journal did not keep.
    pub(crate) irreversible: Vec<IrreversibleEntry>,
}

/// Planned change of the database file, shown by a preview.
pub(crate) fn planned_change(request: &SqlRequest) -> Change {
    let mut change = Change::new(request.display.clone(), ChangeAction::Modify);
    change.size_before = fs::metadata(&request.path)
        .ok()
        .map(|metadata| Size::from_bytes(metadata.len()));
    change
}

/// Runs the statements as one change preserved by `tx`.
pub(crate) fn write(
    request: &SqlRequest,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> Result<WriteOutcome> {
    session::check_file(request)?;
    let files = DatabaseFiles::locate(request)?;
    let before = files.snapshot();
    let preserved = tx.preserve(&files.specs(&before))?;
    let irreversible = files.irreversible(&preserved);
    let mut rows = Vec::new();
    let summary = execute(request, ctx, &mut rows)?;
    let after = files.snapshot();
    let changes = files.changes(&before, &after, summary.wrote);
    if !changes.is_empty() {
        tx.observe_external(&changes)?;
    }
    let mut change = Change::new(request.display.clone(), ChangeAction::Modify);
    change.size_before = before.first().map(|state| Size::from_bytes(state.len()));
    change.size_after = after.first().map(|state| Size::from_bytes(state.len()));
    change.replacements = Some(summary.changes);
    Ok(WriteOutcome {
        rows,
        change,
        irreversible,
    })
}

fn execute(request: &SqlRequest, ctx: &ExecCtx, rows: &mut Vec<Record>) -> Result<Summary> {
    let connection = session::open(request)?;
    let interrupt = connection.get_interrupt_handle();
    let finished = AtomicBool::new(false);
    let result = thread::scope(|scope| {
        scope.spawn(|| watch(ctx, &interrupt, &finished));
        let result = transaction(&connection, request, ctx, rows);
        finished.store(true, Ordering::Release);
        result
    });
    let summary = result.map_err(|error| ctx.checkpoint().err().unwrap_or(error))?;
    connection
        .close()
        .map_err(|(_, e)| error::from_sqlite(&e, request, Site::Transaction))?;
    Ok(summary)
}

/// Interrupts the running statement when the execution is cancelled or its
/// deadline passes.
fn watch(ctx: &ExecCtx, interrupt: &InterruptHandle, finished: &AtomicBool) {
    while !finished.load(Ordering::Acquire) {
        let cancelled = ctx.cancel().wait_timeout(WATCH_INTERVAL).is_some();
        if cancelled || ctx.checkpoint().is_err() {
            interrupt.interrupt();
            return;
        }
    }
}

fn transaction(
    connection: &Connection,
    request: &SqlRequest,
    ctx: &ExecCtx,
    rows: &mut Vec<Record>,
) -> Result<Summary> {
    let site = Site::Transaction;
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| error::from_sqlite(&e, request, site))?;
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
    let summary = match session::run(connection, request, &mut sink) {
        Ok(summary) => summary,
        Err(error) => {
            // A failed ROLLBACK leaves the files to the journal, which
            // restores them when the executor rolls the operation back.
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    };
    connection
        .execute_batch("COMMIT")
        .map_err(|e| error::from_sqlite(&e, request, site))?;
    connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(|e| error::from_sqlite(&e, request, site))?;
    Ok(summary)
}
