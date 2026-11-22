---
command: sql
group: documents
summary: Run SQL statements on an SQLite database file, read-only unless write is set.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sql

## Synopsis

```text
sql <path> <query> [params: <list<any> | map<any>>] [write: <bool>]
```

## Description

`sql` runs the SQL statements of `query` on the SQLite database file `path`
and produces one `row` record per result row. Statements run in order;
statements without result columns produce no records.

Without `write: true` the database is opened read-only and the connection is
set to `query_only`. A statement that changes the database fails with
`E_SQL`, and the error carries the same call with `write: true` as `fix`.
Every statement is prepared and checked before the first row is produced.

With `write: true` all statements run in one `BEGIN IMMEDIATE` transaction.
The transaction is committed when every statement succeeds and rolled back
when one fails; nothing is changed then. After the commit the write-ahead
log is checkpointed into the database file with `TRUNCATE`, so the database
file holds the new state. Statements that control the transaction (`BEGIN`,
`COMMIT`, `END`, `ROLLBACK` without `TO`) are refused; `SAVEPOINT`,
`RELEASE` and `ROLLBACK TO` are allowed. `VACUUM` cannot run inside the
transaction and fails with `E_SQL`.

Every connection is limited to the database file and its journal files.
`ATTACH`, `DETACH` and the pragmas `temp_store_directory` and
`data_store_directory` are refused (`problem`: `attach`, `pragma`), a call of
`load_extension` is refused (`problem`: `extension`), and the connection runs
in the defensive mode of SQLite with untrusted schema. `VACUUM INTO` is
refused because it attaches a second database file. Without `write: true` it
fails with the problem `write_required` like every statement that is not
read-only, and the target file is not created.

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

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | SQLite database file. |
| `query` | `text` | required | SQL statements, separated by `;`. |
| `params` | `list<any> \| map<any>` | `[]` | Values bound to the placeholders of the statements: a list in statement order, or a map from placeholder names to values. |
| `write` | `bool` | `false` | Allow statements that change the database. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `row` |

Fields of `row`:

| Field | Content |
|---|---|
| `path` | the database file |
| `no` | number of the row within its statement, from 1 |
| `cells` | values by column name, in column order; a repeated column name gets the suffix `_2`, `_3`, … |
| `table` | number of the statement, from 1, when `query` holds more than one statement |

Rows are produced while the query runs. A pipeline that uses only the first
rows, or a cancelled execution, interrupts the running statement.

## Effect

| Category | Effect | Reversibility |
|---|---|---|
| change | `read`; `local` with `write: true` | read; with `write: true` reversible when the previous content is preserved |

With `write: true` the operation journal preserves the database file and the
files `-wal`, `-shm` and `-journal` next to it before the first statement
runs. `undo` restores the database and these files to their previous bytes
and removes journal files created by the change. Without space for the copy
the change is made and reported as irreversible with `no_space_for_copy`.
The change record of the database file reports the number of rows inserted,
updated or deleted, triggers included, in `rows_changed`.

A lock held by another connection is awaited for `sql.busy_timeout` (5
seconds by default) and then fails with `E_IN_USE`.

## Notes

A read of a database in write-ahead log mode creates the files `-wal` and
`-shm` when they are missing and the directory is writable, as every SQLite
reader does; the database file itself is not changed.

## Examples

Create a table in an empty database file and return the inserted row:

```nurt example
sql "/home/user/app.db" "create table users(id integer primary key, name text); insert into users(name) values (?) returning id, name" params: ["ada"] write: true
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
| `E_SQL` | a statement cannot be prepared or executed, changes the database without `write: true`, is refused (`problem`: `attach`, `extension`, `pragma`, `transaction`), holds no statement (`problem`: `empty_query`), or `params` does not match the statements (`problem`: `parameter_count`, `parameter_name`, `parameter_type`, `parameter_range`); the problem `write_required` marks a statement that changes the database without `write: true`, `VACUUM INTO` included; `details` holds `statement` and `sqlite_code` | correct the statement, add `write: true`, or pass one value per parameter or one entry per named placeholder |
| `E_MISSING_PARAM` | `path` or `query` is not given | pass both |
| `E_TYPE` | `path`, `query`, `params` or `write` has another type | pass `params` as a list or a map |
| `E_DOC_PARSE` | the file is not an SQLite database, is encrypted or damaged; `details.format` is `sqlite`, `details.sqlite_code` holds the SQLite code when SQLite reported it | pass an SQLite database file |
| `E_NOT_FOUND` | the database file does not exist | check the path; an empty file is an empty database |
| `E_IN_USE` | another connection holds a lock longer than `sql.busy_timeout` | retry later or raise `sql.busy_timeout` |
| `E_IO` | the file cannot be read or written for another reason | see the message |
| `E_PERMISSION` | the file or its directory cannot be read or written | check the permissions |
| `E_NO_SPACE` | the volume is full during the change | free space and run the call again |
| `E_LIMIT` | the statements of a change return more rows than one execution may hold | return fewer rows from `RETURNING` or `SELECT` |
| `E_CANCELLED`, `E_TIMEOUT` | the execution was cancelled or its time limit passed; a change is rolled back | run the call again |

## Related

`query` for JSON and other structured documents, `set` for values at key
paths, `undo` for reverting a change.
