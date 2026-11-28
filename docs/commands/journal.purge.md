---
command: journal.purge
group: journal
summary: Release journal entries and the space they hold.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/journal.purge.md -->
<!-- Responsibility:   Documents journal.purge. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# journal.purge

## Synopsis

```text
journal.purge [name: value]...
```

## Description

`journal.purge` releases journal entries at once, together with the space
their data holds. Without `up_to` and `id` every entry of the selected volumes
is released; `up_to` releases the entries up to and including that operation
number; `id` releases exactly the listed entries and keeps older ones.

Pinned entries are released as well: a pin protects an entry from expiry by
steps, age and size, not from an explicit release. An operation that spans
several volumes is released on all of them, so that no entry is left
partially undoable.

A released entry becomes `expired` with the reason `purge`; a tombstone with
its metrics remains for `history`. Committed, partial and undone entries are
released; entries that are already expired, being reclaimed or corrupt are not
changed. The release is recorded in the audit log of the account; it creates
no journal entry and consumes no step.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `volume` | `path \| enum(all)` | `all` | Volume whose journal is purged, or all volumes. |
| `up_to` | `int` | none | Release all entries up to and including this operation number. |
| `id` | `list<int>` | none | Release only these entries, keeping older ones. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The `effect` record of kind `journal.purge` lists the numbers of the released
entries in `after.released` and their logical size in `after.bytes`;
`summary.released` counts them, and `irreversible` carries `purged` when at
least one entry was released.

## Effect

| Property | Value |
|---|---|
| Category | `journal` |
| Effect | `local` |
| Reversibility | `irreversible` |
| Implementation | `builtin` |

Released entries can no longer be undone; the result carries the
irreversible marker `purged`.

## Examples

Release every entry of every volume:

```nurt example
journal.purge
```

Release the entries up to operation 120 on the volume of a project:

```nurt example
journal.purge volume: "/home/user/project" up_to: 120
```

Release two large entries and keep the rest:

```nurt example
journal.purge id: [131, 134]
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | no journal volume contains `volume` | omit `volume` or name a path on a volume with a journal |
| `E_INVALID_VALUE` | both `up_to` and `id` are given, or `id` lists no entry | give one of them |
| `E_IO` | a journal directory cannot be changed | check the permissions of the journal directory |

## Related

`journal.status`, `journal.limits`, `journal.pin`, `history`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
