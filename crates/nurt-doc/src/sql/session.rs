// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections and statement execution.

use std::fs;
use std::io::Read;
use std::path::Path as FsPath;
use std::sync::Arc;

use indexmap::IndexMap;
use nurt_contracts::error::{Error, Result};
use nurt_contracts::record::Row;
use nurt_contracts::value::{Fields, Path, Record, Value};
use rusqlite::config::DbConfig;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Batch, Connection, OpenFlags, Statement};

use super::error::{self, Site};
use super::guard::Policy;
use super::{SqlParams, SqlRequest, convert, split};

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

/// A configured connection with its authorizer.
pub(super) struct Database {
    connection: Connection,
    policy: Arc<Policy>,
}

impl Database {
    /// Opens the database: read-only unless `request.write` is set, never
    /// creating the file, with extension loading disabled, attached
    /// databases unable to create or change files and the authorizer of
    /// [`Policy`] installed.
    pub(super) fn open(request: &SqlRequest) -> Result<Self> {
        let access = if request.write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let mut connection = Connection::open_with_flags(
            FsPath::new(&request.path),
            access | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| error::from_sqlite(&e, request, Site::Open))?;
        configure(&connection, request).map_err(|e| error::from_sqlite(&e, request, Site::Open))?;
        let policy = Policy::install(&mut connection, request.write)
            .map_err(|e| error::from_sqlite(&e, request, Site::Open))?;
        Ok(Self { connection, policy })
    }

    /// The connection, for the statements that `sql` runs itself.
    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Closes the connection, so that every file handle is released.
    pub(super) fn close(self, request: &SqlRequest) -> Result<()> {
        self.connection
            .close()
            .map_err(|(_, e)| error::from_sqlite(&e, request, Site::Transaction))
    }

    /// Prepares every statement without running it and checks the
    /// statements and the parameter values. Used before a read, where no
    /// statement can change the schema seen by the next one.
    pub(super) fn validate(&self, request: &SqlRequest) -> Result<()> {
        let _scope = self.policy.statements();
        let mut binder = Binder::new(&request.params);
        let mut batch = Batch::new(&self.connection, &request.query);
        let mut number = 0;
        while let Some(statement) = self.next_statement(&mut batch, request, number + 1)? {
            number += 1;
            admit(&statement, number, request)?;
            binder.values(&statement, number)?;
        }
        binder.finish()
    }

    /// Runs the statements in order and passes every result row to `sink`.
    pub(super) fn run(
        &self,
        request: &SqlRequest,
        sink: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<Summary> {
        let _scope = self.policy.statements();
        let start = self.connection.total_changes();
        let multiple = split::statement_count(&request.query) > 1;
        let mut binder = Binder::new(&request.params);
        let mut batch = Batch::new(&self.connection, &request.query);
        let mut summary = Summary::default();
        let mut number = 0;
        while let Some(mut statement) = self.next_statement(&mut batch, request, number + 1)? {
            number += 1;
            admit(&statement, number, request)?;
            summary.wrote |= !statement.readonly();
            for (offset, (value, label)) in
                binder.values(&statement, number)?.into_iter().enumerate()
            {
                let converted = convert::to_sqlite(value, &label)?;
                statement
                    .raw_bind_parameter(offset + 1, converted)
                    .map_err(|e| self.fail(&e, request, number))?;
            }
            let table = multiple.then(|| number.to_string());
            self.emit_rows(&mut statement, number, request, table, sink)?;
        }
        binder.finish()?;
        summary.changes = self.connection.total_changes().saturating_sub(start);
        Ok(summary)
    }

    fn next_statement<'c>(
        &self,
        batch: &mut Batch<'c, '_>,
        request: &SqlRequest,
        number: usize,
    ) -> Result<Option<Statement<'c>>> {
        batch.next().map_err(|e| self.fail(&e, request, number))
    }

    /// Converts the failure of statement `number`; a refusal of the
    /// authorizer is reported with its reason.
    fn fail(&self, error: &rusqlite::Error, request: &SqlRequest, number: usize) -> Error {
        match self.policy.take_denial() {
            Some(denial) => error::denied(&denial, number),
            None => error::from_sqlite(error, request, Site::Statement(number)),
        }
    }

    fn emit_rows(
        &self,
        statement: &mut Statement<'_>,
        number: usize,
        request: &SqlRequest,
        table: Option<String>,
        sink: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<()> {
        let names = unique_names(statement.column_names());
        let mut rows = statement.raw_query();
        let mut no = 0;
        while let Some(row) = rows.next().map_err(|e| self.fail(&e, request, number))? {
            no += 1;
            let mut cells = IndexMap::with_capacity(names.len());
            for (index, name) in names.iter().enumerate() {
                let value = row
                    .get_ref(index)
                    .map_err(|e| self.fail(&e, request, number))?;
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

/// Refuses a statement that changes the database while `write` is false.
/// `sqlite3_stmt_readonly` decides, so `VACUUM` and journal mode changes
/// count as changes as well.
fn admit(statement: &Statement<'_>, number: usize, request: &SqlRequest) -> Result<()> {
    if !request.write && !statement.readonly() {
        return Err(error::read_only(request, Site::Statement(number)));
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

/// Assignment of the values of `params` to the parameters of the
/// statements, in statement order.
struct Binder<'v> {
    params: &'v SqlParams,
    /// List values consumed so far.
    used: usize,
    /// Map entries bound to at least one placeholder, by entry index.
    named: Vec<bool>,
}

impl<'v> Binder<'v> {
    fn new(params: &'v SqlParams) -> Self {
        let named = match params {
            SqlParams::List(_) => Vec::new(),
            SqlParams::Map(fields) => vec![false; fields.len()],
        };
        Self {
            params,
            used: 0,
            named,
        }
    }

    /// Values of the parameters of one statement, in parameter order, each
    /// with the label that names it in errors.
    fn values(
        &mut self,
        statement: &Statement<'_>,
        number: usize,
    ) -> Result<Vec<(&'v Value, String)>> {
        let count = statement.parameter_count();
        match self.params {
            SqlParams::List(values) => {
                if values.len() - self.used < count {
                    return Err(error::missing_params(number, self.used));
                }
                let first = self.used;
                self.used += count;
                Ok((first + 1..)
                    .zip(&values[first..self.used])
                    .map(|(position, value)| (value, position.to_string()))
                    .collect())
            }
            SqlParams::Map(fields) => (1..=count)
                .map(|index| self.named_value(fields, statement.parameter_name(index), number))
                .collect(),
        }
    }

    fn named_value(
        &mut self,
        fields: &'v Fields,
        name: Option<&str>,
        number: usize,
    ) -> Result<(&'v Value, String)> {
        let name = match name {
            Some(name) if !name.starts_with('?') => name,
            other => return Err(error::positional_with_map(number, other.unwrap_or("?"))),
        };
        let (index, value) =
            entry(fields, name).ok_or_else(|| error::missing_name(number, name))?;
        if let Some(used) = self.named.get_mut(index) {
            *used = true;
        }
        Ok((value, format!("`{name}`")))
    }

    fn finish(&self) -> Result<()> {
        match self.params {
            SqlParams::List(values) if self.used != values.len() => {
                Err(error::extra_params(self.used, values.len()))
            }
            SqlParams::List(_) => Ok(()),
            SqlParams::Map(fields) => {
                let unused: Vec<&str> = fields
                    .keys()
                    .zip(&self.named)
                    .filter(|(_, used)| !**used)
                    .map(|(name, _)| name.as_str())
                    .collect();
                if unused.is_empty() {
                    Ok(())
                } else {
                    Err(error::unused_names(&unused))
                }
            }
        }
    }
}

/// The map entry of placeholder `name`: the entry written with the prefix
/// first, then the entry without it.
fn entry<'v>(fields: &'v Fields, name: &str) -> Option<(usize, &'v Value)> {
    fields
        .get_full(name)
        .or_else(|| name.get(1..).and_then(|bare| fields.get_full(bare)))
        .map(|(index, _, value)| (index, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(entries: &[(&str, i64)]) -> Fields {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Value::Int(*value)))
            .collect()
    }

    #[test]
    fn renames_repeated_columns() {
        let names = unique_names(["a", "a", "a_2", "b"].into_iter());
        assert_eq!(names, ["a", "a_2", "a_2_2", "b"]);
    }

    #[test]
    fn counts_list_values() {
        let connection = Connection::open_in_memory().unwrap();
        let one = connection.prepare("select ?").unwrap();
        let two = connection.prepare("select ?, ?").unwrap();
        let params = SqlParams::List(vec![Value::Int(1), Value::Int(2)]);
        let mut binder = Binder::new(&params);
        let first = binder.values(&one, 1).unwrap();
        assert_eq!(first, [(&Value::Int(1), "1".to_owned())]);
        assert!(binder.finish().is_err());
        assert!(binder.values(&two, 2).is_err());
        assert_eq!(binder.values(&one, 3).unwrap()[0].1, "2");
        assert!(binder.finish().is_ok());
    }

    #[test]
    fn binds_map_entries_with_and_without_prefix() {
        let connection = Connection::open_in_memory().unwrap();
        let statement = connection.prepare("select :a, @b, $c, :a").unwrap();
        let params = SqlParams::Map(fields(&[("a", 1), ("@b", 2), ("c", 3), ("b", 9)]));
        let mut binder = Binder::new(&params);
        let values = binder.values(&statement, 1).unwrap();
        let bound: Vec<_> = values
            .iter()
            .map(|(value, label)| ((*value).clone(), label.as_str()))
            .collect();
        assert_eq!(
            bound,
            [
                (Value::Int(1), "`:a`"),
                (Value::Int(2), "`@b`"),
                (Value::Int(3), "`$c`")
            ]
        );
        let unused = binder.finish().err().unwrap();
        assert!(unused.message.contains(": b;"), "{}", unused.message);
    }

    #[test]
    fn refuses_positional_placeholders_with_a_map() {
        let connection = Connection::open_in_memory().unwrap();
        let params = SqlParams::Map(fields(&[("a", 1)]));
        for query in ["select ?", "select ?1"] {
            let statement = connection.prepare(query).unwrap();
            let error = Binder::new(&params).values(&statement, 1).err().unwrap();
            assert_eq!(error.problem.as_deref(), Some("parameter_name"), "{query}");
        }
        let missing = connection.prepare("select :b").unwrap();
        let error = Binder::new(&params).values(&missing, 2).err().unwrap();
        assert_eq!(error.problem.as_deref(), Some("parameter_name"));
    }
}
