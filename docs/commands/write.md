---
command: write
group: core
summary: Write text, bytes or records to a file.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# write

## Synopsis

```text
write [<path>] [<text>] mode: <enum(create|overwrite|append|prepend)> (path: <path> | files: <list<map<any>>>) [name: value]...
```

## Description

`write` writes one file, or a batch of files given in `files` in one
transaction: all files are written or none. The content is `text`, the
content named by `from`, the bytes of `data`, or the input records. `size`
truncates or extends the file without content; `data` with `offset` writes
bytes in place.

`mode` has no default. `create` fails with `E_EXISTS` when the file exists;
`overwrite` replaces the content; `append` adds the text at the end;
`prepend` adds it at the start. Every mode creates a missing file.

A new file is UTF-8 without a byte order mark, with the line endings of the
configuration key `text.new_file_line_ending` (`lf` by default). An existing
file keeps its encoding, its byte order mark and its line endings, unless
`encoding`, `bom` or `line_ending` give other values; text is appended in the
encoding and line endings of the file. A character the encoding cannot
represent fails with `E_ENCODING` at its line and column before anything is
written, with a `convert` call as the fix.

A link in the last component of `path` is followed: the file it names is
written and the link stays a link. A directory is never written. Missing
parent directories are created unless `parents` is `false`; each one is a
target of its own. Content identical to the file is not written and is
reported as `unchanged`.

Input records are written in `format`; `auto` follows the extension of the
file (`.json`, `.yaml`, `.yml`, `.toml`, `.csv`, `.tsv`), otherwise `text`.
`json` writes an indented array of objects; `csv` and `tsv` write a header
row with every field name followed by one row per record; `text` writes one
line per record, the `text` field when the record has one, otherwise the
record as compact JSON. `yaml` and `toml` need the documents group.

A `registry:/` path writes a registry value on Windows, of the type
`value_type` (`string` by default); `mode` is `create` or `overwrite`.

Text written to a file of a known format is checked after the write: a
document format of the documents group, otherwise a language of the code
group. Only syntax errors that the previous content did not have count.
With `validate: reject` such an error rolls the whole write back with
`E_ROLLED_BACK`; with `warn` it is reported as a warning and the write is
kept; `off` skips the check. Bytes, sizes and registry values are not
checked, and without the documents and code groups nothing is checked.

## Privilege

Files that the account may not change (`/etc/hosts`, files of other accounts) are written with `privilege: elevated`. The entry goes to the system journal with the owner, group, mode and access control of the previous content, and `undo` restores them through the same backend.

```nurt
write "/etc/hosts" "127.0.0.1 localhost\n" mode: append privilege: elevated
```

With `privilege: auto` the stage runs with the account and is repeated once with `elevated` when a target fails with `E_PERMISSION` or `E_PRIVILEGE`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `mode` | `enum(create\|overwrite\|append\|prepend)` | required | Write mode; there is no default. |
| `path` | `path` | one of `target` | File to write. |
| `text` | `text` | none | Text to write. |
| `from` | `pipeline \| stream<any> \| path` | none | Pipeline, named result or file whose content is written. |
| `encoding` | `text` | `"auto"` | Text encoding; auto keeps the encoding of an existing file. |
| `line_ending` | `enum(lf\|crlf\|cr\|keep)` | `keep` | Line endings of the written text. |
| `bom` | `bool` | as in the existing file | Write a byte order mark. |
| `parents` | `bool` | `true` | Create missing parent directories. |
| `ensure_newline` | `bool` | `false` | End the file with a line ending. |
| `format` | `enum(auto\|json\|yaml\|toml\|csv\|tsv\|text)` | `auto` | Serialization of written records; auto follows the file extension. |
| `files` | `list<map<any>>` | one of `target` | Batch of files written in one transaction, each with path, text and optional mode. |
| `value_type` | `enum(string\|dword\|qword\|multi_string\|expand_string\|binary)` | none | Type of a registry value; only for registry: paths. |
| `file_mode` | `text` | none | Permissions of a new file, such as "0644". |
| `size` | `size` | none | Truncate or extend the file to this size. |
| `data` | `bytes` | none | Bytes to write. |
| `offset` | `int` | none | Byte offset at which data is written. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Roll back a write that adds a new syntax error to a known format. |
| `if_match` | `text \| map<text>` | none | Expected etag of the file, or a map from path to etag for files. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none`, `any` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Create a file with three lines:

```nurt example
write "/home/user/app/Dockerfile" "FROM alpine:3.20\nRUN apk add --no-cache curl\nCMD [\"sh\"]\n" mode: create
```

```text expected
+ /home/user/app/Dockerfile
```

Append a line to an existing file of 120 bytes:

```nurt example
write "/home/user/app/.gitignore" "dist/\n" mode: append
```

```text expected
120b → 126b
```

Replace a file that uses CRLF line endings; the new text keeps them:

```nurt example
write "/home/user/app/config.ini" "[main]\nport = 8080\n" mode: overwrite
```

Write two files in one transaction; the missing directory `docs` is created:

```nurt example
write files: [{path: "/home/user/app/a.txt", text: "a\n"}, {path: "/home/user/app/docs/b.txt", text: "b\n"}] mode: create
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `mode`, or both `path` and `files`, are missing | give `mode: create`, `overwrite`, `append` or `prepend` |
| `E_EXISTS` | `mode: create` and the file exists | use `mode: overwrite` or `mode: append` |
| `E_PARAM_CONFLICT` | two sources of content, `path` with `files`, or `offset` without `data` | give one source of content |
| `E_ENCODING` | a character the encoding of the file cannot represent | `convert` the file to UTF-8 first, or give `encoding` |
| `E_STALE` | the file changed after its etag was read, or after the plan | read the file again and use the new etag |
| `E_NOT_FOUND` | a parent directory is missing and `parents` is `false`, or a link names a missing entry | set `parents: true` or create the target |
| `E_INVALID_VALUE` | the path is a directory, `file_mode` is not octal, or `value_type` is given for a file | correct the value |
| `E_ROLLED_BACK` | the written text adds a syntax error and `validate` is `reject` | correct the text or pass `validate: warn` |
| `E_UNSUPPORTED` | `format: yaml` or `toml` without the documents group | load the group or use `format: json` |
| `E_OS_ONLY` | a `registry:/` path outside Windows | write files instead |
| `E_LIMIT` | a file read for the change is larger than the text limit | write with `data` and `offset`, or `size` |

## Related

`create`, `replace`, `insert`, `edit`, `convert`, `read`, `undo`.
