---
command: history
group: core
summary: List operations of the journal.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# history

## Synopsis

```text
history [<id>] [name: value]...
```

## Description

`history` lists the operations recorded in the journal of the account, latest
first. Reads, previews and failed operations rolled back before they took
effect have no entry and are not listed.

Each operation has a state:

| State | Meaning |
|---|---|
| `available` | the operation can be undone |
| `partial` | only some items can be undone: the others were released to free disk space or were undone individually |
| `undone` | the operation was undone |
| `expired` | the operation can no longer be undone; its metrics remain listed |

Operations still being recorded or rolled back are not listed. An operation
that failed its integrity check is listed as `expired`.

The filters combine. `path` selects operations that changed the file,
comparing paths as the volume does (letter case and Unicode normalisation on
case-insensitive volumes); expired operations keep no item list and never
match `path` or `deleted`. Without `limit` at most 20 operations are listed.

With `details: true` and `id`, an operation that is not in the journal or is
still being recorded fails with `E_JOURNAL_NOOP` instead of an empty result.
With `details: true` every listed record also carries `items`, the targets of
the operation; the list is empty for an `expired` operation, and without
`details` the field is absent.

The result states whether an operation can be undone, never until when;
retention limits and the state of the journal are reported by
`journal.status`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `int` | none | Operation to describe. |
| `limit` | `int` | `20` | Number of operations listed. |
| `state` | `enum(any\|available\|partial\|expired\|undone)` | `any` | State of the operations to list. |
| `before` | `int` | none | List only operations older than this one. |
| `since` | `int` | none | List only operations newer than this one. |
| `path` | `path` | none | List only operations that touched this file. |
| `deleted` | `bool` | `false` | List only operations that removed files. |
| `details` | `bool` | `false` | Include the changes and differences of each operation. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `operation` |

Each item of `items` has the fields `path`, `action` (the action on the target,
for example `create`, `modify` or `rename`), `mode` (how the previous content
is kept, for example `clone` or `none`), `before_hash` and `after_hash` (absent
when there was no content), `undone` and `released`.

The text form of the result shows the columns `id`, `time`, `kind`, `state`,
`files`, `bytes` and `summary`, and `items` with `details: true`. The other
fields of the operation (`action`, `irreversible`, `bulk`, `session`,
`undo_of`, `redo_of`, `undo_scope`, `ast_hash`, `manifest_sha256`, `chain`
and `pinned`) stay in the JSON form and appear in the text with verbosity
`detailed` or when the request names them in `fields`.

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the latest operations:

```nurt example
history
```

List the operations that changed one file:

```nurt example
history path: "/home/user/app/main.go"
```

Describe operation 18; the call fails when the operation is not in the journal:

```nurt example
history 18 details: true
```

List the operations that removed files and can still be undone:

```nurt example
history deleted: true state: available
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOURNAL_NOOP` | with `details: true`, operation `id` is not in the journal or is still being recorded | list operations without `id` |
| `E_INVALID_VALUE` | `limit` or an operation number is below 1 | give a number of at least 1 |
| `E_IO` | the journal index cannot be read | check the state directory of the account |

## Related

`undo`, `redo`, `journal.status`, `journal.pin`.
