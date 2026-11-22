---
command: redo
group: core
summary: Apply an undone operation again.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# redo

## Synopsis

```text
redo [<id>]
```

## Description

`redo` applies again the changes reversed by an undo. `id` is the number of
the undo operation, as listed by `history`; without it the latest available
undo of the session (of the account without a session) is selected.

The undo operation is reversed as a whole. For an undo of one file
(`undo file:`) or of selected items (`undo N paths:`) only the items that undo
restored are applied again.

Before anything is written, every item is compared with the state recorded by
the undo, with the same check as `undo`. A later operation on the same files
makes the comparison fail, so a redo never overwrites newer work: the call
fails with `E_JOURNAL_CONFLICT` and nothing changes. `redo` has no
`on_conflict` parameter; to apply the changes anyway, undo the undo operation
with `undo N on_conflict: force`.

The undo operation becomes `undone`. The operations it undid stay `undone`;
their changes are now held by the redo operation, which is undone with `undo`
like any other operation.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `int` | the latest undone operation | Undone operation to apply again. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Apply the latest undo of the session again:

```nurt example
redo
```

Apply undo operation 42 again:

```nurt example
redo 42
```

Reverse a redo by undoing it:

```nurt example
undo 43
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_JOURNAL_NOOP` | there is no undo to redo, the operation is not an undo, or it is already redone | list operations with `history` |
| `E_JOURNAL_EXPIRED` | the undo operation expired; nothing changed | none; the changes can no longer be applied from the journal |
| `E_JOURNAL_CONFLICT` | the files changed after the undo; nothing changed | inspect the differences in `conflicts` |
| `E_JOURNAL_PARTIAL` | items of the undo operation were released for space; nothing changed | `undo N on_conflict: partial` with the undo operation |
| `E_JOURNAL_CORRUPT` | the undo operation failed its integrity check | `nurt journal salvage` |
| `E_UNSUPPORTED` | an item cannot be restored by the journal (a rename across volumes or over an existing entry); nothing changed | apply the change again with the original command |
| `E_STALE` | an item changed while the redo ran; the redo was rolled back | repeat the redo |

## Related

`undo`, `history`, `help skill: "journal"`.
