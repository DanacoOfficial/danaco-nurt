---
command: undo
group: core
summary: Undo operations from the journal.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/undo.md -->
<!-- Responsibility:   Documents undo. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# undo

## Synopsis

```text
undo [<id>] [name: value]...
```

## Description

`undo` restores the state that preceded one or more operations recorded in
the journal. It repairs a mistake; it does not replace checking the result of
an operation before relying on it.

Exactly one form selects the operations:

| Form | Operations undone | Scope recorded |
|---|---|---|
| no selection | the latest available operation of the session (of the account without a session) that is not itself an undo | `op` |
| `id` | operation `id`, all its items; a bulk operation is undone as a whole | `op` |
| `to` | every available operation newer than `to` in the session, latest first, as one transaction | `to` |
| `file` | every available operation that changed the file, newer than `since` or all of them, for that file only; the file returns to its state before the oldest of them | `file` |
| `id` with `paths` | the listed items of operation `id` only | `paths` |

An undo of an undo is applied again with `redo`; the default selection never
picks an undo. Operations of other sessions inside a `to` range are skipped
and listed in `skipped` unless `all_sessions` is true.

Reports name the operation `op`, while the parameter that selects it is `id`.
A call written `undo op: N` or `undo operation: N` is corrected to `undo id: N`
(rule L06) and runs; the correction is listed in the result.

Before anything is written, every item of the whole selection is compared
with the state the journal recorded after the operation: content hash, kind,
size and permission bits, absence of deleted entries, the content preserved
for overwritten and deleted files, and later operations that changed the same
items. A difference changes nothing and consumes no operation number;
`on_conflict` decides what happens instead:

| `on_conflict` | Behaviour |
|---|---|
| `report` | fails with `E_JOURNAL_CONFLICT` listing every difference; fails with `E_JOURNAL_PARTIAL` when an operation has items released for space |
| `force` | undoes anyway; the current content of every item is kept in the new journal entry; differences are listed in `forced`; released items are skipped as with `partial` |
| `include_later` | also undoes, latest first, the later operations that changed the same items, and the later operations of those, until no later change remains; fails with `E_JOURNAL_EXPIRED` when one of them expired; with `to` it behaves as `report`, since a range is already undone latest first |
| `partial` | undoes the remaining items of operations with items released for space and lists the released items in `unavailable`; differences are reported as with `report` |
| `abort` | fails with `E_JOURNAL_CONFLICT` reporting only the number of differences, or with `E_JOURNAL_PARTIAL` |

An operation is in the state `partial` when some of its items were released
to free disk space or were undone individually with `file` or `paths`; the
items still kept remain undoable. Items undone individually are restored only
once; an operation whose items are all undone becomes `undone`.

Inside a `to` or `file` range, an undo whose operation lies in the same range
cancels it, and neither is undone; a `redo` of such an undo is undone.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `int` | the latest operation | Operation to undo. |
| `to` | `int` | none | Undo every operation newer than this one, latest first, in one transaction. |
| `file` | `path` | none | Undo every change of this file since the given point. |
| `since` | `int \| instant` | none | Operation or time from which changes of file are undone. |
| `paths` | `list<path>` | none | Undo only the changes of these paths within the operation. |
| `all_sessions` | `bool` | `false` | Include operations of other sessions; only with to. |
| `on_conflict` | `enum(report\|force\|include_later\|partial\|abort)` | `report` | Behaviour when later changes touch the same files. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

A preview of `undo id` and `undo id paths:` lists the items the undo would
restore; the other forms are previewed without items.

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |
| Reversibility conditions | An undo can be reversed with redo. |

## Examples

Undo the latest operation of the session:

```nurt example
undo
```

Undo operation 18 together with the later operations that changed the same
files:

```nurt example
undo 18 on_conflict: include_later
```

Return a file to its state before operation 12:

```nurt example
undo file: "/home/user/app/config.toml" since: 12
```

Undo two items of a bulk removal:

```nurt example
undo 21 paths: ["/home/user/app/a.go", "/home/user/app/b.go"]
```

Undo everything done after operation 30 in this session:

```nurt example
undo to: 30
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOURNAL_NOOP` | the operation does not exist, is already undone or still in progress, a path is not an item of the operation, or nothing is left to undo | list operations with `history` |
| `E_INVALID_VALUE` | the arguments mix forms: more than one of `id`, `to` and `file`, `since` without `file`, `paths` without `id`, an empty `paths`, or `all_sessions` without `to` | use one form of the synopsis |
| `E_JOURNAL_EXPIRED` | an operation of the selection expired; nothing changed; `expired` lists them | select later operations |
| `E_JOURNAL_CONFLICT` | items changed after the operation; nothing changed; `conflicts` lists every difference | use one of the programs in `fix`, or `on_conflict: force` or `include_later` |
| `E_JOURNAL_PARTIAL` | items of the operation were released for space; nothing changed | `on_conflict: partial` |
| `E_JOURNAL_CORRUPT` | an operation failed its integrity check; its data is kept for salvage | `nurt journal salvage` |
| `E_UNSUPPORTED` | an item cannot be restored by the journal (a rename across volumes or over an existing entry); nothing changed | restore the item from the data listed by `history N details: true`, or undo the other items with `paths` |
| `E_STALE` | an item changed while the undo ran; the undo was rolled back | repeat the undo |
| `E_ROLLBACK_FAILED` | an undo that failed could not restore every item; `failed` lists them | restore the listed paths |

## Related

`redo`, `history`, `journal.pin`, `journal.status`, `help skill: "journal"`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
