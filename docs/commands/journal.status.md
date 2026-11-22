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
raises lower values from any level; a raised limit has the source `floor`.

| Field | Meaning |
|---|---|
| `active` | the journal keeps data on the volume |
| `entries` | entries that can be undone, partial entries included |
| `bytes` | logical bytes held by the journal |
| `limits` | the limits `steps`, `age` and `size`, each with `name`, `value`, `source`, `scope` and `floor` |
| `volume` | mount point of the volume |
| `oldest` | commit time of the oldest entry that can be undone |
| `oldest_id` | number of that entry |
| `reason` | why the journal is off on the volume, for example `service_down` or `network_volume`; absent when it is on |

`source` names the level that set the value: `default`, `system`, `user`,
`project`, `config_file`, `environment`, `session` (set by `journal.limits`
for the session, then `scope` is `session`) or `floor`. `floor` gives the
protection floor of `steps` and `age`; it is absent when only the project
minimum applies. `unlimited` is the value of a limit that is off.

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
journal.status |> filter active == false
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | no journal volume contains `volume` | omit `volume` or name a path on a volume with a journal |
| `E_CONFIG` | a configuration file has an invalid journal setting | correct the key named in the error |
| `E_IO` | a journal directory cannot be read | check the permissions of the journal directory |

## Related

`journal.limits`, `journal.space`, `journal.purge`, `history`.
