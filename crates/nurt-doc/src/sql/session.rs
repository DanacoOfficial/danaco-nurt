// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections and statement execution.

use std::fs;
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Row;
use nurt_contracts::value::{Fields, Path, Record, Value};
use rusqlite::config::DbConfig;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Batch, Connection, OpenFlags, Statement};

use super::error::{self, Site};
use super::guard::Policy;
use super::names::{self, DatabaseName};
use super::split::StatementKind;
use super::{SqlParams, SqlRequest, convert, split, target};

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
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
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

    /// Actions of the call that the journal cannot undo, as the authorizer
    /// recorded them while it prepared the statements.
    pub(super) fn effects(&self) -> super::guard::Effects {
        self.policy.effects()
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
            self.bind(&mut binder, &mut statement, number, request)?;
            let table = multiple.then(|| number.to_string());
            self.emit_rows(&mut statement, number, request, table, sink)?;
        }
        binder.finish()?;
        summary.changes = self.connection.total_changes().saturating_sub(start);
        Ok(summary)
    }

    /// Binds the next values of `binder` to `statement`.
    fn bind<'v>(
        &self,
        binder: &mut Binder<'v>,
        statement: &mut Statement<'_>,
        number: usize,
        request: &SqlRequest,
    ) -> Result<()> {
        for (offset, (value, label)) in binder.values(statement, number)?.into_iter().enumerate() {
            let converted = convert::to_sqlite(value, &label)?;
            statement
                .raw_bind_parameter(offset + 1, converted)
                .map_err(|e| self.fail(&e, request, number))?;
        }
        Ok(())
    }

    /// Runs the statements of a change.
    ///
    /// Statements run in one transaction, except those SQLite refuses inside
    /// one: `ATTACH` and the pragmas run in their place with the transaction
    /// committed before them, `DETACH` and `VACUUM` run after the other
    /// statements. `BEGIN`, `COMMIT` and `END` are merged with the
    /// transaction of the call, and `ROLLBACK` ends the call with
    /// `E_ROLLED_BACK`. The write-ahead logs of every database are
    /// checkpointed at the end.
    pub(super) fn run_write(
        &self,
        request: &SqlRequest,
        files: &mut dyn Files,
        sink: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<Summary> {
        let mut script = Script {
            open: false,
            deferred: Vec::new(),
        };
        let result = self.run_script(request, files, &mut script, sink);
        if result.is_err() && script.open {
            // A failed ROLLBACK leaves the files to the journal, which
            // restores them when the executor rolls the operation back.
            let _ = self.connection.execute_batch("ROLLBACK");
        }
        result
    }

    fn run_script<'c>(
        &'c self,
        request: &SqlRequest,
        files: &mut dyn Files,
        script: &mut Script<'c>,
        sink: &mut dyn FnMut(Record) -> Result<()>,
    ) -> Result<Summary> {
        let start = self.connection.total_changes();
        let kinds = split::statement_kinds(&request.query);
        let multiple = kinds.len() > 1;
        let mut binder = Binder::new(&request.params);
        let mut batch = Batch::new(&self.connection, &request.query);
        let mut summary = Summary::default();
        let mut number = 0;
        while let Some(mut statement) = self.next_statement(&mut batch, request, number + 1)? {
            number += 1;
            let Some(kind) = kinds.get(number - 1).copied() else {
                return Err(split_mismatch(number, kinds.len()));
            };
            self.bind(&mut binder, &mut statement, number, request)?;
            let table = multiple.then(|| number.to_string());
            match kind {
                StatementKind::Begin | StatementKind::Commit => {}
                StatementKind::Rollback => return Err(self.rolled_back(script, number, files)),
                StatementKind::Attach => {
                    self.commit(script, request)?;
                    self.attach(statement, number, request, files)?;
                }
                StatementKind::VacuumInto => {
                    self.commit(script, request)?;
                    self.vacuum_into(statement, number, request, files)?;
                }
                StatementKind::Detach => {
                    self.commit(script, request)?;
                    self.emit_rows(&mut statement, number, request, None, &mut |_| Ok(()))?;
                }
                StatementKind::Vacuum => {
                    summary.wrote = true;
                    script.deferred.push((number, statement));
                }
                StatementKind::Pragma => {
                    self.commit(script, request)?;
                    summary.wrote = true;
                    self.emit_rows(&mut statement, number, request, table, sink)?;
                }
                _ => {
                    self.begin(script, request)?;
                    summary.wrote |= !statement.readonly();
                    self.emit_rows(&mut statement, number, request, table, sink)?;
                }
            }
        }
        binder.finish()?;
        if number != kinds.len() {
            return Err(split_mismatch(number + 1, kinds.len()));
        }
        self.commit(script, request)?;
        for (number, mut statement) in std::mem::take(&mut script.deferred) {
            self.emit_rows(&mut statement, number, request, None, sink)?;
        }
        self.checkpoint_all(request)?;
        summary.changes = self.connection.total_changes().saturating_sub(start);
        Ok(summary)
    }

    fn begin(&self, script: &mut Script<'_>, request: &SqlRequest) -> Result<()> {
        if !script.open {
            self.connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| error::from_sqlite(&e, request, Site::Transaction))?;
            script.open = true;
        }
        Ok(())
    }

    fn commit(&self, script: &mut Script<'_>, request: &SqlRequest) -> Result<()> {
        if script.open {
            self.connection
                .execute_batch("COMMIT")
                .map_err(|e| error::from_sqlite(&e, request, Site::Transaction))?;
            script.open = false;
        }
        Ok(())
    }

    /// Ends the call for a `ROLLBACK` statement: the open transaction is
    /// rolled back and the executor restores the journaled files. Effects
    /// that cannot be undone and were reported before are named in the error.
    fn rolled_back(&self, script: &mut Script<'_>, number: usize, files: &dyn Files) -> Error {
        if script.open {
            let _ = self.connection.execute_batch("ROLLBACK");
            script.open = false;
        }
        let mut kept = files.notes();
        kept.extend(self.policy.effects().describe());
        let message = if kept.is_empty() {
            format!("statement {number} is ROLLBACK: the call is rolled back and changes nothing")
        } else {
            format!(
                "statement {number} is ROLLBACK: the call is rolled back; effects that cannot be undone remain: {}",
                kept.join("; ")
            )
        };
        let error = Error::new(ErrorCode::RolledBack, message)
            .with_problem("rolled_back")
            .with_detail("statement", serde_json::json!(number));
        if kept.is_empty() {
            error
        } else {
            error.with_detail("irreversible", serde_json::json!(kept))
        }
    }

    /// Runs `ATTACH` outside the transaction. The file is computed, resolved
    /// like a path argument and journaled before the statement; the statement
    /// then runs with the resolved name, so SQLite opens the file that the
    /// journal preserved. A file that cannot be told is reported as an effect
    /// that cannot be undone, before the change.
    fn attach(
        &self,
        mut statement: Statement<'_>,
        number: usize,
        request: &SqlRequest,
        files: &mut dyn Files,
    ) -> Result<()> {
        let sql = statement.expanded_sql().unwrap_or_default();
        let parts = target::attach_parts(&sql);
        let routed = match &parts {
            Some(parts) => self.route(&parts.expression, number, request, files)?,
            None => None,
        };
        let sink = &mut |_| Ok(());
        match (parts, routed) {
            (Some(parts), Some(name)) => {
                let mut rewritten = self
                    .connection
                    .prepare(&format!("ATTACH DATABASE ?1 AS {}", parts.alias))
                    .map_err(|e| self.fail(&e, request, number))?;
                rewritten
                    .raw_bind_parameter(1, name.as_str())
                    .map_err(|e| self.fail(&e, request, number))?;
                self.emit_rows(&mut rewritten, number, request, None, sink)
            }
            _ => self.emit_rows(&mut statement, number, request, None, sink),
        }
    }

    /// Runs `VACUUM [schema] INTO` in its place, after the earlier statements
    /// committed, with the new file resolved and journaled first.
    fn vacuum_into(
        &self,
        mut statement: Statement<'_>,
        number: usize,
        request: &SqlRequest,
        files: &mut dyn Files,
    ) -> Result<()> {
        let sql = statement.expanded_sql().unwrap_or_default();
        let parts = target::vacuum_parts(&sql);
        let routed = match &parts {
            Some(parts) => self.route(&parts.expression, number, request, files)?,
            None => None,
        };
        let sink = &mut |_| Ok(());
        match (parts, routed) {
            (Some(parts), Some(name)) => {
                let schema = parts
                    .schema
                    .map(|schema| format!("{schema} "))
                    .unwrap_or_default();
                let mut rewritten = self
                    .connection
                    .prepare(&format!("VACUUM {schema}INTO ?1"))
                    .map_err(|e| self.fail(&e, request, number))?;
                rewritten
                    .raw_bind_parameter(1, name.as_str())
                    .map_err(|e| self.fail(&e, request, number))?;
                self.emit_rows(&mut rewritten, number, request, None, sink)
            }
            _ => self.emit_rows(&mut statement, number, request, None, sink),
        }
    }

    /// Computes the file that `expression` names, resolves it, declares it to
    /// the journal and returns the name SQLite must receive. `None` when the
    /// database is not a file (memory), or when the file cannot be told; in
    /// the second case the effect is reported as one that cannot be undone.
    fn route(
        &self,
        expression: &str,
        number: usize,
        request: &SqlRequest,
        files: &mut dyn Files,
    ) -> Result<Option<String>> {
        let value = match target::literal_of(expression) {
            Some(value) => Some(value),
            None => self.evaluate(expression, request)?,
        };
        let Some(value) = value else {
            files.untracked(
                number,
                "the statement names a file that could not be resolved before it ran, so the file is not preserved",
            );
            return Ok(None);
        };
        match names::database_name(&value) {
            DatabaseName::Memory => Ok(None),
            DatabaseName::Unknown => {
                files.untracked(
                    number,
                    "the statement names a URI whose file cannot be told, so the file is not preserved",
                );
                Ok(None)
            }
            DatabaseName::File { path, uri, query } => {
                let file = names::absolute(request.base.as_deref(), &path)?;
                files.watch(file.clone(), &file.to_string_lossy())?;
                Ok(Some(names::written(&file, uri, query.as_deref())))
            }
        }
    }

    /// Value of `expression` computed with a `SELECT`; `None` when it is not
    /// text or cannot be computed. An interrupt is an error.
    fn evaluate(&self, expression: &str, request: &SqlRequest) -> Result<Option<String>> {
        match self
            .connection
            .query_row(&format!("SELECT ({expression})"), [], |row| {
                row.get::<_, Option<String>>(0)
            }) {
            Ok(value) => Ok(value),
            Err(rusqlite::Error::SqliteFailure(failure, message))
                if failure.code == rusqlite::ErrorCode::OperationInterrupted =>
            {
                Err(error::from_sqlite(
                    &rusqlite::Error::SqliteFailure(failure, message),
                    request,
                    Site::Transaction,
                ))
            }
            Err(_) => Ok(None),
        }
    }
    /// Name and file of every database of the connection.
    fn schemas(&self, request: &SqlRequest) -> Result<Vec<(String, String)>> {
        let failed = |e: rusqlite::Error| error::from_sqlite(&e, request, Site::Transaction);
        let mut statement = self
            .connection
            .prepare("PRAGMA database_list")
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })
            .map_err(failed)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(failed)
    }

    /// Moves the write-ahead log of every database into its file.
    fn checkpoint_all(&self, request: &SqlRequest) -> Result<()> {
        for (name, _) in self.schemas(request)? {
            if name == "temp" {
                continue;
            }
            let sql = format!(
                "PRAGMA \"{}\".wal_checkpoint(TRUNCATE)",
                name.replace('"', "\"\"")
            );
            self.connection
                .query_row(&sql, [], |_| Ok(()))
                .map_err(|e| error::from_sqlite(&e, request, Site::Transaction))?;
        }
        Ok(())
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

/// Allows the SQL function `load_extension` on a connection opened for a
/// change; a read-only connection refuses it.
#[allow(
    unsafe_code,
    reason = "rusqlite marks enabling extension loading unsafe; only the SQL function is enabled"
)]
fn enable_extensions(connection: &Connection) -> rusqlite::Result<()> {
    // SAFETY: the call switches on the SQL function `load_extension` of this
    // connection; the effects of a loaded library are the responsibility of
    // the caller of `sql`, who asked for the change, and the report marks
    // them as unknown and irreversible.
    unsafe { connection.load_extension_enable() }
}

/// The files that a change touches besides the database of the call; the
/// journal preserves each of them before the statements change it.
pub(super) trait Files {
    /// Declares the database file `file` and its `-wal`, `-shm` and
    /// `-journal` files; a file declared before is ignored.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the journal when it cannot preserve the
    /// files.
    fn watch(&mut self, file: PathBuf, label: &str) -> Result<()>;

    /// Notes an effect on files that are not known before the statement
    /// runs; the effect cannot be undone.
    fn untracked(&mut self, number: usize, what: &str);

    /// Descriptions of the effects reported so far that cannot be undone.
    fn notes(&self) -> Vec<String>;
}

/// Error for statements that the lexer and SQLite split differently.
fn split_mismatch(number: usize, expected: usize) -> Error {
    Error::new(
        ErrorCode::Sql,
        format!(
            "statement {number} does not match the {expected} statements the query was read as; split the query at its semicolons"
        ),
    )
    .with_problem("statement_split")
}

/// State of the statements of one change.
struct Script<'c> {
    /// A transaction is open.
    open: bool,
    /// `VACUUM` statements that run after the others.
    deferred: Vec<(usize, Statement<'c>)>,
}

fn configure(connection: &Connection, request: &SqlRequest) -> rusqlite::Result<()> {
    connection.busy_timeout(request.busy_timeout)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, !request.write)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_db_config(
        DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE,
        request.write,
    )?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, request.write)?;
    if request.write {
        enable_extensions(connection)?;
    }
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
