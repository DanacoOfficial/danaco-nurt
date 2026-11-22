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
history [id: int] [limit: int] [state: any|available|partial|expired|undone] [before: int] [since: int] [path: path] [deleted: bool] [details: bool]
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

With `details: true` every listed operation includes its items: address,
action, how the previous state is kept, the content hashes before and after,
and whether the item was undone individually or released for space.

The result states whether an operation can be undone, never until when;
retention limits and the state of the journal are reported by
`journal.status`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `int` | | Operation to describe. |
| `limit` | `int` | `20` | Number of operations listed. |
| `state` | `enum(any\|available\|partial\|expired\|undone)` | `any` | State of the operations to list. |
| `before` | `int` | | List only operations older than this one. |
| `since` | `int` | | List only operations newer than this one. |
| `path` | `path` | | List only operations that touched this file. |
| `deleted` | `bool` | `false` | List only operations that removed files. |
| `details` | `bool` | `false` | Include the changes and differences of each operation. |

## Input and output

Input: `none`. Output: `operation` records — number, commit time, kind
(command and action), one-line summary, state, number of items, bytes
changed, irreversible markers, bulk flag, session, the operations an undo
undid (`undo_of`), the undo a redo reversed (`redo_of`), the scope of an undo,
the program hash, the manifest hash, the hash chain link and whether the
operation is pinned.

## Effect

Category `journal`, effect `read`. `history` changes nothing and consumes no
operation number.

## Examples

List the latest operations:

```nurt example
history
```

List the operations that changed one file:

```nurt example
history path: "/home/user/app/main.go"
```

Describe operation 18 with its items:

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
| `E_JOURNAL_NOOP` | with `details: true`, the operation is not in the journal or is still being recorded | list operations without `id` |
| `E_JOURNAL_CORRUPT` | with `details: true`, the operation failed its integrity check | `nurt journal salvage` |
| `E_IO` | the journal index cannot be read | check the state directory of the account |

## Related

`undo`, `redo`, `journal.status`, `journal.pin`.
