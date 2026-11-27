---
command: sql
group: documents
summary: Run SQL statements on an SQLite database file; a statement that changes anything makes the call a change.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sql

## Synopsis

```text
sql <path> <query> [name: value]...
```

## Description

`sql` runs the SQL statements of `query` on the SQLite database file `path`
and produces one `row` record per result row. Statements run in order;
statements without result columns produce no records.

A call whose statements only read (`SELECT`, `VALUES`, `EXPLAIN`, a `WITH`
without `INSERT`, `UPDATE` or `DELETE`, a pragma that only reads) opens the
database read-only and sets the connection to `query_only`. A call with a
statement that changes anything opens the database for writing and is an
effect; no refusal asks for `write: true`. The statements that make a call a
change are every other statement, a pragma that sets a value or acts, and a
call of `load_extension`. A query that is not a literal text is a change.
`write: true` stays as an optional explicit form. `write: false` keeps the
call read-only: a statement that changes the database then fails with
`E_SQL` (`problem`: `write_required`), and the error carries the same call
without `write: false` as `fix`. Every statement of a read is prepared and
checked before the first row is produced.

A change journals the database file and its `-wal`, `-shm` and `-journal`
files before the first statement runs. The statements run in one
`BEGIN IMMEDIATE` transaction. The transaction is committed when every
statement succeeds and rolled back when one fails; nothing is changed then.
After the commit the write-ahead log of every database is checkpointed into
its file with `TRUNCATE`, so the files hold the new state.

| Statement | Rule |
|---|---|
| `BEGIN`, `COMMIT`, `END` | merged with the transaction of the call; they change nothing |
| `ROLLBACK` | rolls the call back and ends it with `E_ROLLED_BACK`; the database and the journal stay as before |
| `SAVEPOINT`, `RELEASE`, `ROLLBACK TO` | run inside the transaction |
| `ATTACH` | allowed; runs outside the transaction (the transaction is committed before it and opened again for the next change). The file is journaled before the first statement that changes it; a file that does not exist is created and `undo` removes it. The path is the one SQLite resolves: absolute, or relative to the working directory of the process |
| `DETACH` | allowed; runs after the other statements, or before the next `ATTACH` |
| `VACUUM`, `VACUUM schema` | run outside the transaction automatically, after the other statements; the database file is journaled first |
| `VACUUM INTO` | allowed; the new file is journaled as created when it is a text literal or a parameter, so `undo` removes it; any other expression makes the call irreversible for that file (`external_effect`) |
| pragmas that set a value or act | run outside the transaction, in their place, so `journal_mode`, `foreign_keys` and `temp_store_directory` work; `temp_store_directory` and `data_store_directory` are allowed |
| `load_extension` | allowed; the report marks the call with `external_effect`, because the effects of the extension are unknown and cannot be undone |

The connection runs in the defensive mode of SQLite with untrusted schema.
A call with `write: false` refuses `ATTACH`, `DETACH`, the directory pragmas
(`problem`: `attach`, `pragma`) and `load_extension` (`problem`:
`extension`).

`path` must name an existing file. An empty file is an empty database, as in
SQLite. A file that does not start with the SQLite header, such as an
encrypted database, fails with `E_DOC_PARSE`.

Parameter values bind to the placeholders of the statements. `params` is
either a list or a map.

A list binds in statement order: each statement takes as many values as it
has parameters. Positional (`?`), numbered (`?2`) and named (`:id`, `@id`,
`$id`) placeholders take their value by their parameter index; a named
placeholder has the index of its first occurrence. The number of values must
equal the number of parameters of all statements.

A map binds named placeholders by name. A key is written with or without the
placeholder prefix, so `id` and `:id` both bind `:id`. Every named placeholder
needs an entry and every entry must be used by a placeholder. A positional
placeholder (`?`, `?2`) cannot be bound from a map. A violation fails with
`E_SQL` and the problem `parameter_name`.

| Value type | SQLite value |
|---|---|
| `null` | `NULL` |
| `bool` | `INTEGER` 0 or 1 |
| `int`, `size` (bytes), `duration` (milliseconds) | `INTEGER` |
| `number` | `REAL` |
| `text`, `path`, `glob`, `key`, `enum`, `decimal`, `date` | `TEXT` |
| `instant` | `TEXT` in ISO 8601 form |
| `bytes` | `BLOB` |

| SQLite value | Cell type |
|---|---|
| `INTEGER` | `int` |
| `REAL` | `number`; an infinity is the text `Inf` or `-Inf` |
| `TEXT` | `text`; text that is not UTF-8 is `bytes` |
| `BLOB` | `bytes` |
| `NULL` | `null` |

A read of a database in write-ahead log mode creates the files `-wal` and
`-shm` when they are missing and the directory is writable, as every SQLite
reader does; the database file itself is not changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | SQLite database file. |
| `query` | `text` | required | SQL statements. |
| `params` | `list<any> \| map<any>` | `[]` | Values bound to the placeholders of the statements: a list for positional placeholders, a map from names to values for named placeholders. |
| `write` | `bool` | none | Optional: a statement that changes the database opens it for writing without it. write: false keeps the call read-only. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `row` |

Fields of `row`:

Rows are produced while the query runs. A pipeline that uses only the first
rows, or a cancelled execution, interrupts the running statement.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `builtin` |
| Reversibility conditions | A call that changes the database copies the database file and its journal files first, and undo restores them; the files of ATTACH and VACUUM INTO are preserved or removed the same way. Without space for the copy, and for the effects of load_extension, the change is irreversible. |

| Action | Effect | Reversibility | Note |
|---|---|---|---|
| `write` | `local` | `reversible_if_preserved` | Applies when write is true, and without write when a statement changes anything. |

With a change the operation journal preserves the database file and the
files `-wal`, `-shm` and `-journal` next to it before the first statement
runs. `undo` restores the database and these files to their previous bytes
and removes journal files created by the change. Without space for the copy
the change is made and reported as irreversible with `no_space_for_copy`.
The change record of the database file reports the number of rows inserted,
updated or deleted, triggers included, in `rows_changed`.

A lock held by another connection is awaited for `sql.busy_timeout` (5
seconds by default) and then fails with `E_IN_USE`.

## Examples

Create a table in an empty database file and return the inserted row:

```nurt example
sql "/home/user/app.db" "create table users(id integer primary key, name text); insert into users(name) values (?) returning id, name" params: ["ada"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

List the tables of a database:

```nurt example
sql "/home/user/app.db" "select name from sqlite_master where type = 'table' order by name"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Bind named parameters from a map:

```nurt example
sql "/home/user/app.db" "select :low as low, :high as high" params: {"low": 1, "high": 9}
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Compute values from bound parameters with numbered placeholders:

```nurt example
sql "/home/user/app.db" "select ?2 - ?1 as span, upper(?3) as label" params: [5, 12, "week"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_SQL` | a statement cannot be prepared or executed, changes the database with `write: false`, is refused with `write: false` (`problem`: `attach`, `extension`, `pragma`), holds no statement (`problem`: `empty_query`), or `params` does not match the statements (`problem`: `parameter_count`, `parameter_name`, `parameter_type`, `parameter_range`); the problem `write_required` marks a statement that changes the database with `write: false`, `VACUUM INTO` included; `details` holds `statement` and `sqlite_code` | correct the statement, remove `write: false`, or pass one value per parameter or one entry per named placeholder |
| `E_ROLLED_BACK` | a statement `ROLLBACK` ended the call; `details.statement` holds its number and nothing was changed | remove the `ROLLBACK` statement to keep the changes |
| `E_MISSING_PARAM` | `path` or `query` is not given | pass both |
| `E_TYPE` | `path`, `query`, `params` or `write` has another type | pass `params` as a list or a map |
| `E_DOC_PARSE` | the file is not an SQLite database, is encrypted or damaged; `details.format` is `sqlite`, `details.sqlite_code` holds the SQLite code when SQLite reported it | pass an SQLite database file |
| `E_NOT_FOUND` | the database file does not exist | check the path; an empty file is an empty database |
| `E_IN_USE` | another connection holds a lock longer than `sql.busy_timeout` | retry later or raise `sql.busy_timeout` |
| `E_IO` | the file cannot be read or written for another reason | see the message |
| `E_PERMISSION` | the file or its directory cannot be read or written | check the permissions |
| `E_NO_SPACE` | the volume is full during the change | free space and run the call again |
| `E_LIMIT` | the statements of a change return more rows than one execution may hold | return fewer rows from `RETURNING` or `SELECT` |
| `E_CANCELLED` | the execution was cancelled or its time limit passed; a change is rolled back | run the call again |
| `E_TIMEOUT` | the execution was cancelled or its time limit passed; a change is rolled back | run the call again |

## Related

`query` for JSON and other structured documents, `set` for values at key
paths, `undo` for reverting a change.
