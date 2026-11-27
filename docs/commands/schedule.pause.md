---
command: schedule.pause
group: schedule
summary: Pause schedules without removing them.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/schedule.pause.md -->
<!-- Responsibility:   Documents schedule.pause. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# schedule.pause

## Synopsis

```text
schedule.pause <id>... [name: value]...
```

## Description

`schedule.pause` marks the selected active schedules as paused in one
operation of the journal and returns one `effect` record per selected
schedule. A paused schedule keeps its definition and history and makes no
runs; run times that pass while it is paused are not made up. A run in
progress is not stopped.

Only schedules in the state `active` change. Paused, completed and expired
schedules are reported with `action: unchanged` and their state in `before`,
and no file is written for them.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `list<text>` | none | Schedules to pause. |
| `all` | `bool` | `false` | Pause all schedules. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The effect record has `kind: schedule.pause`, `target: schedule:<id>`,
`action` (`pause` or `unchanged`), `before` with the previous state and
`after` with the new state.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Pause one schedule:

```nurt example
schedule.pause "s2"
```

Pause two schedules in one operation:

```nurt example
schedule.pause ["s2", "s4"]
```

Pause every schedule, for example before maintenance of the device:

```nurt example
schedule.pause all: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | Neither `id` nor `all: true` is given. | Give the identifiers or `all: true`. |
| `E_PARAM_CONFLICT` | `id` and `all: true` are given together. | Keep one of them. |
| `E_NOT_FOUND` | Some identifiers do not exist; `actual` lists them. Nothing is changed. | Correct the identifiers; `schedule.list` shows the existing ones. |
| `E_INVALID_VALUE` | An identifier contains characters other than letters, digits, `-` and `_`. | Give identifiers such as `s3`. |
| `E_IO` | The schedule store cannot be written. | Check the permissions of the state directory. |

## Related

`schedule.resume` resumes paused schedules; `schedule.list state: paused`
lists them.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
