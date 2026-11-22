---
command: journal.status
group: journal
summary: Report the number of entries, space used, oldest entry and active limits with their sources.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.status

## Synopsis

```text
journal.status [volume: path]
```

## Description

`journal.status` reports the retention state of the journal: one
`journal_state` record per volume, or for the volume that contains `volume`.
It is the only command that reports retention state; the results of other
commands state whether an operation can be undone, never for how long.

The effective limits are resolved from seven configuration levels, in
increasing precedence: built-in defaults, the system file, the user file, the
project file `.nurt.toml` (only for a trusted project), the file named by
`NURT_CONFIG`, the `NURT_JOURNAL_*` environment variables and run-time
settings of `journal.limits`. Every limit is reported with the level that set
it. A protection floor (`journal.floor.max_ops`, `journal.floor.max_age`)
raises lower values from any level; the record reports the floor, the level
that set it and whether a limit was raised.

| Field | Meaning |
|---|---|
| `volume` | mount point of the volume |
| `entries_total` | entries of the volume, expired entries included |
| `entries_available` | entries that can be undone, partial entries included |
| `entries_expired` | tombstones of expired entries kept for `history` (`journal.history_keep`) |
| `bytes_held` | logical bytes held by the journal |
| `bytes_physical` | estimated physical bytes, block sharing of clones taken into account |
| `oldest` | number and commit time of the oldest available entry |
| `limits` | effective `max_ops`, `max_age`, `max_size` (with the budget in bytes), `oversize`, `age_clock` and `max_op_size`, each with its source level |
| `floor` | protection floor in force and its level; absent when only the project minimum applies |
| `inactive` | why the journal is off on the volume, for example `service_down` or `network_volume` |
| `over_limit` | entries over a limit that wait to be reclaimed |
| `notes` | `journal_config_ignored` and `trust_config_ignored` for project files, once per file; `raised_to_floor` for raised limits |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `volume` | `path` | | Only the journal of the volume that contains this path. |

## Input and output

| Input | Output |
|---|---|
| none | `journal_state` |

## Effect

| Category | Effect | Reversibility |
|---|---|---|
| journal | read | read |

## Examples

Report the journal of every volume:

```nurt example
journal.status
```

Report the journal of the volume that holds a project:

```nurt example
journal.status volume: "/home/user/project"
```

List the volumes where the journal is off:

```nurt example
journal.status |> where inactive != null
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | no journal volume contains `volume` | omit `volume` or name a path on a volume with a journal |
| `E_CONFIG` | a configuration file has an invalid journal setting | correct the key named in the error |
| `E_IO` | a journal directory cannot be read | check the permissions of the journal directory |

## Related

`journal.limits`, `journal.space`, `journal.purge`, `history`.
