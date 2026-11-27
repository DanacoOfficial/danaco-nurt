---
command: schedule.resume
group: schedule
summary: Resume paused schedules.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/schedule.resume.md -->
<!-- Responsibility:   Documents schedule.resume. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# schedule.resume

## Synopsis

```text
schedule.resume <id>... [name: value]...
```

## Description

`schedule.resume` makes the selected paused schedules active again in one
operation of the journal and returns one `effect` record per selected
schedule. The next run time counts from the time of the resumption: a
schedule with `every: 10m` resumed at 15:07 next runs at 15:17. A run that
waited in the queue of `overlap: queue` before the pause is dropped. A single
time (`at`) that passed during the pause is handled by `on_missed`.

Only schedules in the state `paused` change. Active, completed and expired
schedules are reported with `action: unchanged` and their state in `before`,
and no file is written for them.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `list<text>` | none | Schedules to resume. |
| `all` | `bool` | `false` | Resume all paused schedules. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

The effect record has `kind: schedule.resume`, `target: schedule:<id>`,
`action` (`resume` or `unchanged`), `before` with the previous state and
`after` with the new state.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Resume one schedule:

```nurt example
schedule.resume "s2"
```

Resume two schedules in one operation:

```nurt example
schedule.resume ["s2", "s4"]
```

Resume every paused schedule and list the next run times:

```nurt example
schedule.resume all: true
schedule.list state: active |> select id label next_run
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

`schedule.pause` pauses schedules; `schedule.list` shows their state and next
run time.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
