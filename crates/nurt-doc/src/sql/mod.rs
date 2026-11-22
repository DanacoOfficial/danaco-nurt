// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Access to SQLite databases.
//!
//! A database is opened read-only unless the call allows changes. The
//! authorizer of every connection refuses statements that reach files other
//! than the database and its own journal files: `ATTACH`, `DETACH` and the
//! pragmas that set directories for other files. It also refuses the
//! `load_extension` function, attached databases cannot create or change
//! files, and the connection runs in defensive mode. Changes run in one
//! `BEGIN IMMEDIATE` transaction after the database file and its journal
//! files are preserved by the operation journal.

mod convert;
mod error;
mod guard;
mod journal;
mod read;
mod session;
mod split;
mod write;

use std::path::PathBuf;
use std::time::Duration;

use nurt_contracts::value::{Fields, Value};

pub(crate) use read::read_rows;
pub(crate) use session::check_file;
pub(crate) use split::statement_count;
pub(crate) use write::{planned_change, write};

/// Values bound to the placeholders of the statements.
#[derive(Debug, Clone)]
pub(crate) enum SqlParams {
    /// Values consumed in statement order, one per parameter.
    List(Vec<Value>),
    /// Values by placeholder name, written with or without the prefix `:`,
    /// `@` or `$`.
    Map(Fields),
}

/// One call of `sql`, with arguments already checked.
#[derive(Debug, Clone)]
pub(crate) struct SqlRequest {
    /// Database file.
    pub(crate) path: PathBuf,
    /// Canonical text of the database path, used in records and errors.
    pub(crate) display: String,
    /// SQL statements.
    pub(crate) query: String,
    /// Values bound to the parameters of the statements.
    pub(crate) params: SqlParams,
    /// Statements may change the database.
    pub(crate) write: bool,
    /// Longest wait for a lock held by another connection.
    pub(crate) busy_timeout: Duration,
}
