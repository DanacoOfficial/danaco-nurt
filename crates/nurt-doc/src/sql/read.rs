// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading rows as a lazy stream.
//!
//! The statements run on a worker thread that owns the connection and sends
//! rows through a bounded channel, so a large result never stays in memory
//! and a consumer that stops early (`first`) stops the query. Dropping the
//! stream or cancelling the execution interrupts the running statement.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{ExecCtx, RecordStream};
use nurt_contracts::value::Record;
use rusqlite::{Connection, InterruptHandle};

use super::{SqlRequest, session};

/// Rows produced ahead of the consumer.
const ROW_BUFFER: usize = 256;

/// Interval between checks for cancellation while waiting for a row.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Runs read-only statements and returns their rows as a stream.
///
/// The database is opened and every statement is prepared and checked
/// before this function returns, so errors in the statements and in the
/// number of parameters are reported before the first row.
pub(crate) fn read_rows(request: SqlRequest, ctx: &ExecCtx) -> Result<RecordStream<'_>> {
    session::check_file(&request)?;
    let connection = session::open(&request)?;
    session::validate(&connection, &request)?;
    let interrupt = connection.get_interrupt_handle();
    let (sender, receiver) = mpsc::sync_channel(ROW_BUFFER);
    thread::Builder::new()
        .name("nurt-sql".to_owned())
        .spawn(move || produce(&connection, &request, &sender))
        .map_err(|e| {
            Error::new(
                ErrorCode::Limit,
                format!(
                    "no thread is available to run the query: {e}; retry with fewer parallel jobs"
                ),
            )
        })?;
    Ok(RecordStream::new(Rows {
        receiver,
        interrupt,
        ctx,
        done: false,
    }))
}

fn produce(connection: &Connection, request: &SqlRequest, sender: &SyncSender<Result<Record>>) {
    let mut sink = |record: Record| {
        sender
            .send(Ok(record))
            .map_err(|_| Error::new(ErrorCode::Cancelled, "the rows are no longer read"))
    };
    if let Err(error) = session::run(connection, request, &mut sink) {
        // The consumer may already be gone; the error then has no reader.
        let _ = sender.send(Err(error));
    }
}

struct Rows<'a> {
    receiver: Receiver<Result<Record>>,
    interrupt: InterruptHandle,
    ctx: &'a ExecCtx,
    done: bool,
}

impl Rows<'_> {
    fn stop(&mut self) {
        if !self.done {
            self.done = true;
            self.interrupt.interrupt();
        }
    }
}

impl Iterator for Rows<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            if let Err(error) = self.ctx.checkpoint() {
                self.stop();
                return Some(Err(error));
            }
            match self.receiver.recv_timeout(POLL_INTERVAL) {
                Ok(Ok(record)) => return Some(Ok(record)),
                Ok(Err(error)) => {
                    self.done = true;
                    return Some(Err(error));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.done = true,
            }
        }
        None
    }
}

impl Drop for Rows<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
