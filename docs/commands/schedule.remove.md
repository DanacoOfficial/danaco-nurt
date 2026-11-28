---
command: schedule.remove
group: schedule
summary: Remove schedules.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/schedule.remove.md -->
<!-- Responsibility:   Documents schedule.remove. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# schedule.remove

## Synopsis

```text
schedule.remove <id>... [name: value]...
```

## Description

`schedule.remove` deletes the definitions of the selected schedules in one
operation of the journal and returns one `effect` record per schedule. Either
`id` or `all: true` selects the schedules; every identifier must exist.

Removing a schedule stops its runs in progress: the programs of the action
receive `terminate` and, after `run.kill_grace`, `kill`. A run waiting in the
queue of `overlap: queue` is dropped. The stopped runs are listed in
`after.stopped_jobs` of the effect record, and the effect is marked
irreversible with `process_stopped`, because a stopped program cannot be
resumed.

The run history of a removed schedule stays in the run state and remains
visible in `schedule.history`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `list<text>` | none | Schedules to remove. |
| `all` | `bool` | `false` | Remove all schedules. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The effect record has `kind: schedule.remove`, `target: schedule:<id>`,
`action: remove`, `before` with the state before the removal and `after` with
`state: removed` and, when runs were stopped, `stopped_jobs`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

After `undo` the restored schedule counts its next run time from the time of
the restoration, so run times that passed while it was removed are not made
up.

## Examples

Remove one schedule:

```nurt example
schedule.remove "s1"
```

Remove two schedules in one operation:

```nurt example
schedule.remove ["s1", "s2"]
```

Remove every schedule:

```nurt example
schedule.remove all: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `id` nor `all: true` is given. | Give the identifiers or `all: true`. |
| `E_PARAM_CONFLICT` | `id` and `all: true` are given together. | Keep one of them. |
| `E_NOT_FOUND` | Some identifiers do not exist; `actual` lists them. Nothing is removed. | Correct the identifiers; `schedule.list` shows the existing ones. |
| `E_INVALID_VALUE` | An identifier contains characters other than letters, digits, `-` and `_`. | Give identifiers such as `s3`. |
| `E_IO` | The schedule store cannot be written. | Check the permissions of the state directory. |

## Related

`schedule.pause` suspends a schedule without removing it. `schedule.list`
shows the schedules and `schedule.history` their past runs.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
