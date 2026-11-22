---
command: insert
group: core
summary: Insert text before or after lines, matches or nodes, or at the start or end of files.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# insert

## Synopsis

```text
insert <text> in: <path> [name: value]...
line|match|node|entry |> insert <text> [name: value]...
```

## Description

`insert` adds lines to text files. The source form inserts into the file
`in`: before or after `line` with `position: before` or `after`, and at the
start or the end of the file with `position: start` or `end`. Without
`line`, `before` inserts at the start and `after` at the end of the file.

The stage form inserts at every input record. A `line` or `match` record
names its line, and `before` or `after` places the text before or after that
line; `start` and `end` insert once at the start or end of every file named
by the records. An `entry` record names a whole file: `before` and `start`
insert at its start, `after` and `end` at its end. Several records of one
line insert the text once.

All insertions into one file are placed against the file as read: the line
numbers of the input do not move because of earlier insertions, and
insertions at the same place keep the order of the input. The inserted
lines take the line endings of the file. Text inserted at the end of a file
without a final line break leaves the file without one unless
`ensure_newline` is set. A file keeps its encoding and byte order mark.
Empty input changes nothing and is reported as `nothing_matched`. A line
outside the file fails with `E_OUT_OF_RANGE` and changes nothing.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `text` | `text` | required | Text to insert. |
| `in` | `path` | required | File in the source form. |
| `line` | `int` | none | Line number in the source form. |
| `position` | `enum(before\|after\|start\|end)` | `after` | Place of the text relative to the line, match or file. |
| `ensure_newline` | `bool` | `false` | End the inserted text with a line ending. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Roll back a change that adds a new syntax error to a known format. |
| `if_match` | `text` | none | Expected etag of the file. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `line`, `match`, `node`, `entry` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Insert an import before the first line of a file:

```nurt example
insert "use std::fmt;" in: "/home/user/app/src/lib.rs" line: 1 position: before
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/lib.rs", "type": "file", "content_utf8": "pub mod parse;\n" }
] }
```

Insert a comment before the line of every match of a search:

```nurt example
search "fn main" "/home/user/app/src/main.rs" |> insert "// Entry point of the program." position: before
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "fn helper() {}\n\nfn main() {\n    helper();\n}\n" }
] }
```

Append a line at the end of a file:

```nurt example
insert "export PATH=\"$HOME/bin:$PATH\"" in: "/home/user/.profile" position: end
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/.profile", "type": "file", "content_utf8": "umask 022\n" }
] }
```

Add a header comment to every listed file:

```nurt example
find "/home/user/app/src" glob: "*.rs" |> insert "// Part of the parser." position: start
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/a.rs", "type": "file", "content_utf8": "fn a() {}\n" },
  { "path": "/home/user/app/src/b.rs", "type": "file", "content_utf8": "fn b() {}\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OUT_OF_RANGE` | the line lies outside the file | give a line from 1 to the number of lines |
| `E_MISSING_PARAM` | the source form has no `in` | name the file with `in` |
| `E_STALE` | the file changed since it was read, or does not have the `if_match` tag | read the file again and repeat the change |
| `E_BINARY` | the file is binary | insert into text files only |
| `E_ENCODING` | the encoding of the file cannot represent the text | convert the file with `convert` first |
| `E_ROLLED_BACK` | the insertion added a syntax error and `validate` is `reject` | correct the text or pass `validate: warn` |

## Related

`edit`, `replace`, `write`, `lines`, `search`.
