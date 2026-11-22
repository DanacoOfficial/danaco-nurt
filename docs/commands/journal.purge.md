---
command: journal.purge
group: journal
summary: Release journal entries and the space they hold.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.purge

## Synopsis

```text
journal.purge [volume: path | all] [up_to: int] [id: list<int>]
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
its metrics remains for `history`. The release is recorded in the operation
index; it creates no journal entry and consumes no step. Entries that are not
available (already expired, undone partially by space reclaim, or corrupt) are
not changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `volume` | `path \| enum(all)` | `all` | Volume whose journal is purged, or all volumes. |
| `up_to` | `int` | | Release all entries up to and including this operation number. |
| `id` | `list<int>` | | Release only these entries, keeping older ones. |

## Input and output

| Input | Output |
|---|---|
| none | `effect` |

The result lists the numbers of the released entries and their logical size
in bytes.

## Effect

| Category | Effect | Reversibility |
|---|---|---|
| journal | local | irreversible |

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
| `E_IO` | a journal directory cannot be changed | check the permissions of the journal directory |

## Related

`journal.status`, `journal.limits`, `journal.pin`, `history`.
