---
command: schedule.list
group: schedule
summary: List schedules with their next run and state.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.list

## Synopsis

```text
schedule.list [state: <state>]
schedule.list <id>
```

## Description

`schedule.list` returns one `schedule` record per schedule, ordered by
identifier. Without parameters it lists every schedule of the account; `state`
keeps the schedules in one state, and `id` returns one schedule.

The state of a schedule is `paused` while it is paused, `completed` after
`max_runs` runs started by run times, `expired` after its expiry time or after
a missed single time, and `active` otherwise. `active` is `true` only for an
active schedule while the background service runs; otherwise `reason` gives
the cause: `service_not_running`, `paused`, `completed` or `expired`.

`next_run`, `last_run`, `runs` (runs started by run times) and `last_result`
come from the run state kept by the background service. A schedule that the
service has not yet seen shows the next run time computed from its trigger.
Times are instants; the text form shows them with the offset of the time zone
of the schedule.

The command reads the schedule store and changes nothing.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `id` | `text` | none, positional 1 | Schedule to show, for example `s3`. |
| `state` | `enum(all\|active\|paused\|completed\|expired)` | `all` | Only schedules in this state. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `schedule.list [state: …]` | none | `schedule` records, one per schedule |
| `schedule.list <id>` | none | one `schedule` record |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read`: nothing is changed, so there is nothing to undo |
| External programs | none |

## Examples

List all schedules:

```nurt example
schedule.list
```

List the paused schedules:

```nurt example
schedule.list state: paused
```

Show one schedule:

```nurt example
schedule.list "s3"
```

List the labels and next run times of active schedules that do not run:

```nurt example
schedule.list state: active |> where not active |> select label, next_run, reason
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | No schedule has the identifier `id`. `actual` lists the identifier. | List the schedules with `schedule.list` and use one of their identifiers. |
| `E_INVALID_VALUE` | `id` is not a schedule identifier or `state` is unknown. | Give an identifier such as `s3` and one of the listed states. |
| `E_FORMAT` | `<state>/schedules/state.json` is damaged. | Restart the background service; it rewrites the file. |
| `E_IO` | The schedule store cannot be read. | Check the permissions of the state directory. |

## Related

`schedule.history` lists past runs of the schedules; `schedule.create`,
`schedule.pause`, `schedule.resume` and `schedule.remove` change them.
