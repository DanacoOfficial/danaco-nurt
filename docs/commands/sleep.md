---
command: sleep
group: system
summary: Wait for a duration.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/sleep.md -->
<!-- Responsibility:   Documents sleep. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# sleep

## Synopsis

```text
sleep <duration>
```

## Description

`sleep` blocks the program for the given time and then returns without
output. It starts a pipeline and accepts no input.

The wait is performed on the clock of the execution and is interrupted by
cancellation and by the time limit of the program. A cancelled program
returns `E_CANCELLED` within 100 ms of the signal. A wait of `0s` returns at
once.

A wait that reaches the time limit of the program ends at the limit with
`E_TIMEOUT`. The error carries the requested time in `expected` and the time
waited in `actual`, both in milliseconds. When the program does not set
`timeout`, its time limit is raised to the longest `sleep` it contains, so a
wait longer than the default limit of `run` is not cut short; an explicit
`timeout` of the program always applies.

The command changes nothing. It opens no journal operation and has no entry
in the operation history. The time waited is reported as a progress event with
the waited milliseconds as `done` and the requested milliseconds as `total`.
A check of the program (`nurt_check`) reports `waits: true` and the requested
duration without waiting.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `duration` | `duration` | required | Time to wait. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `none` |

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |
| Waits | always blocks the call |

## Examples

Wait for half a second:

```nurt example
sleep 500ms
```

Wait for two seconds:

```nurt example
sleep 2s
```

Create a schedule whose action waits for two seconds, then run the action once
outside its times:

```nurt example
schedule.create "pause" every: 10m action: (sleep 2s)
schedule.run "s1"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | The duration is negative or its end is beyond the range of the clock. | Give a non-negative duration such as `500ms` or `2s`. |
| `E_CANCELLED` | The execution was cancelled before or during the wait. | None; the cancellation was requested by the caller. |
| `E_TIMEOUT` | The time limit of the program ended the wait. `expected` holds the requested milliseconds and `actual` the milliseconds waited. | Set a longer `timeout` for the program or a shorter duration. |

## Related

`schedule.create` and `schedule.run` use `sleep` in the action of a schedule.
`run` starts a program and waits for it with the `wait` parameter. The `system`
group page lists the other commands of the group.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
