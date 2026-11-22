// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections and statement execution.

use std::fs;
use std::io::Read;
use std::path::Path as FsPath;

use indexmap::IndexMap;
use nurt_contracts::error::Result;
use nurt_contracts::record::Row;
use nurt_contracts::value::{Path, Record, Value};
use rusqlite::config::DbConfig;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Batch, Connection, OpenFlags, Statement};

use super::error::{self, Site};
use super::{SqlRequest, convert, guard};

/// First bytes of every SQLite database file.
const HEADER: &[u8] = b"SQLite format 3\0";

/// Totals of one run of the statements.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// A statement that changes the database ran.
    pub(crate) wrote: bool,
    /// Rows inserted, updated or deleted, triggers included.
    pub(crate) changes: u64,
}

/// Checks that the database file exists and holds an SQLite database or
/// nothing; an empty file is an empty database, as in SQLite.
///
/// The check runs before SQLite opens the file, so that a file of another
/// format is reported instead of being treated as a new database.
pub(crate) fn check_file(request: &SqlRequest) -> Result<()> {
    let metadata = fs::metadata(&request.path).map_err(|e| error::file_error(&e, request))?;
    if metadata.is_dir() {
        return Err(error::doc_parse(request, "the path is a directory"));
    }
    let mut header = Vec::with_capacity(HEADER.len());
    fs::File::open(&request.path)
        .and_then(|file| file.take(HEADER.len() as u64).read_to_end(&mut header))
        .map_err(|e| error::file_error(&e, request))?;
    if header.is_empty() || header == HEADER {
        return Ok(());
    }
    Err(
        error::doc_parse(request, "the file does not start with the SQLite header")
            .with_problem("not_sqlite"),
    )
}

/// Opens a configured connection: read-only unless `request.write` is set,
/// never creating the file, with extension loading disabled and attached
/// databases unable to create or change files.
pub(super) fn open(request: &SqlRequest) -> Result<Connection> {
    let access = if request.write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(
        FsPath::new(&request.path),
        access | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| error::from_sqlite(&e, request, Site::Open))?;
    configure(&connection, request).map_err(|e| error::from_sqlite(&e, request, Site::Open))?;
    Ok(connection)
}

fn configure(connection: &Connection, request: &SqlRequest) -> rusqlite::Result<()> {
    connection.busy_timeout(request.busy_timeout)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false)?;
    if !request.write {
        connection.pragma_update(None, "query_only", true)?;
    }
    Ok(())
}

/// Prepares every statement without running it and checks the statements
/// and the number of parameter values. Used before a read, where no
/// statement can change the schema seen by the next one.
pub(super) fn validate(connection: &Connection, request: &SqlRequest) -> Result<()> {
    let mut params = Params::new(&request.params);
    let mut batch = Batch::new(connection, &request.query);
    let mut number = 0;
    while let Some(statement) = next_statement(&mut batch, request, number + 1)? {
        number += 1;
        admit(&statement, number, request)?;
        params.skip(statement.parameter_count(), number)?;
    }
    params.finish()
}

/// Runs the statements in order and passes every result row to `sink`.
pub(super) fn run(
    connection: &Connection,
    request: &SqlRequest,
    sink: &mut dyn FnMut(Record) -> Result<()>,
) -> Result<Summary> {
    let start = connection.total_changes();
    let multiple = guard::statement_count(&request.query) > 1;
    let mut params = Params::new(&request.params);
    let mut batch = Batch::new(connection, &request.query);
    let mut summary = Summary::default();
    let mut number = 0;
    while let Some(mut statement) = next_statement(&mut batch, request, number + 1)? {
        number += 1;
        admit(&statement, number, request)?;
        summary.wrote |= !statement.readonly();
        params.bind(&mut statement, number, request)?;
        let table = multiple.then(|| number.to_string());
        emit_rows(&mut statement, number, request, table, sink)?;
    }
    params.finish()?;
    summary.changes = connection.total_changes().saturating_sub(start);
    Ok(summary)
}

fn next_statement<'c>(
    batch: &mut Batch<'c, '_>,
    request: &SqlRequest,
    number: usize,
) -> Result<Option<Statement<'c>>> {
    batch
        .next()
        .map_err(|e| error::from_sqlite(&e, request, Site::Statement(number)))
}

fn admit(statement: &Statement<'_>, number: usize, request: &SqlRequest) -> Result<()> {
    let text = statement
        .expanded_sql()
        .ok_or_else(|| error::unreadable(number))?;
    if let Some(denial) = guard::check(&text, request.write) {
        return Err(error::denied(&denial, number));
    }
    if !request.write && !statement.readonly() {
        return Err(error::read_only(request, Site::Statement(number)));
    }
    Ok(())
}

fn emit_rows(
    statement: &mut Statement<'_>,
    number: usize,
    request: &SqlRequest,
    table: Option<String>,
    sink: &mut dyn FnMut(Record) -> Result<()>,
) -> Result<()> {
    let names = unique_names(statement.column_names());
    let site = Site::Statement(number);
    let mut rows = statement.raw_query();
    let mut no = 0;
    while let Some(row) = rows
        .next()
        .map_err(|e| error::from_sqlite(&e, request, site))?
    {
        no += 1;
        let mut cells = IndexMap::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|e| error::from_sqlite(&e, request, site))?;
            cells.insert(name.clone(), convert::from_sqlite(value));
        }
        sink(Record::from(Row {
            path: Path::new(request.display.clone()),
            no,
            cells,
            table: table.clone(),
        }))?;
    }
    Ok(())
}

/// Column names as cell names; a repeated name gets the suffix `_2`, `_3`, …
/// so that no cell is lost.
fn unique_names<'n>(columns: impl Iterator<Item = &'n str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for column in columns {
        let mut candidate = column.to_owned();
        let mut suffix = 2;
        while names.contains(&candidate) {
            candidate = format!("{column}_{suffix}");
            suffix += 1;
        }
        names.push(candidate);
    }
    names
}

/// Parameter values consumed in statement order.
struct Params<'v> {
    values: &'v [Value],
    used: usize,
}

impl<'v> Params<'v> {
    fn new(values: &'v [Value]) -> Self {
        Self { values, used: 0 }
    }

    fn skip(&mut self, count: usize, number: usize) -> Result<()> {
        if self.values.len() - self.used < count {
            return Err(error::missing_params(number, self.used));
        }
        self.used += count;
        Ok(())
    }

    fn bind(
        &mut self,
        statement: &mut Statement<'_>,
        number: usize,
        request: &SqlRequest,
    ) -> Result<()> {
        let count = statement.parameter_count();
        let first = self.used;
        self.skip(count, number)?;
        for (offset, value) in self.values[first..self.used].iter().enumerate() {
            let converted = convert::to_sqlite(value, first + offset + 1)?;
            statement
                .raw_bind_parameter(offset + 1, converted)
                .map_err(|e| error::from_sqlite(&e, request, Site::Statement(number)))?;
        }
        Ok(())
    }

    fn finish(&self) -> Result<()> {
        if self.used == self.values.len() {
            Ok(())
        } else {
            Err(error::extra_params(self.used, self.values.len()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_repeated_columns() {
        let names = unique_names(["a", "a", "a_2", "b"].into_iter());
        assert_eq!(names, ["a", "a_2", "a_2_2", "b"]);
    }

    #[test]
    fn counts_parameter_values() {
        let values = [Value::Int(1), Value::Int(2)];
        let mut params = Params::new(&values);
        assert!(params.skip(1, 1).is_ok());
        assert!(params.finish().is_err());
        assert!(params.skip(2, 2).is_err());
        assert!(params.skip(1, 2).is_ok());
        assert!(params.finish().is_ok());
    }
}
